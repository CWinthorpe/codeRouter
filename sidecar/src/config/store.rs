//! Atomic, file-locked JSON configuration store.
//!
//! All reads go through [`read_locked`] which acquires a shared lock, and all
//! writes go through [`atomic_write`] which writes to a temp file and renames
//! it into place. This ensures concurrent processes never see a half-written
//! config file.
//!
//! Each public function returns a [`Result`] that boxes the error so callers
//! don't need to juggle multiple error types.

use std::fs;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use std::os::unix::fs::PermissionsExt;

use crate::config::models::{AppConfig, Group, Provider};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Returns the coderouter configuration directory (`~/.config/coderouter/`).
///
/// Falls back to `./coderouter` when the OS config directory cannot be
/// determined (e.g. in minimal containers).
fn config_dir() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("coderouter");
    path
}

/// Creates the coderouter config directory if it does not already exist.
fn ensure_config_dir() -> Result<()> {
    let dir = config_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    Ok(())
}

/// Appends `name` to the config directory path, producing a full file path.
fn config_file(name: &str) -> PathBuf {
    let mut path = config_dir();
    path.push(name);
    path
}

/// Writes `content` to `path` atomically.
///
/// The write proceeds by:
/// 1. Writing to a temp file suffixed with the current PID,
/// 2. acquiring an exclusive flock,
/// 3. flushing and fsync-ing,
/// 4. setting 0600 permissions,
/// 5. renaming into the final location.
///
/// If the write fails at any step the temp file is removed so no partial
/// artefacts remain.
fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let tmp_path = path.with_extension(format!("tmp.{}", std::process::id()));

    // Ensure the parent directory exists before opening the temp file
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    let write_result = (|| {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_path)?;

        file.lock_exclusive()?;
        file.write_all(content.as_bytes())?;
        file.flush()?;
        file.sync_all()?;

        // Owner-only permissions keep API keys private on multi-user systems
        let mut perms = file.metadata()?.permissions();
        perms.set_mode(0o600);
        file.set_permissions(perms)?;

        file.unlock()?;

        fs::rename(&tmp_path, path)?;

        // rename preserves permissions on most Unix systems, but be explicit
        if let Ok(metadata) = fs::metadata(path) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(path, perms)?;
        }

        Ok(())
    })();

    // Clean up the temp file if anything went wrong
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }

    write_result
}

/// Reads the entire contents of `path` under a shared file lock.
///
/// The shared lock guarantees that a concurrent writer using
/// [`atomic_write`] won't swap the file out from under us.
fn read_locked(path: &Path) -> Result<String> {
    let mut file = fs::OpenOptions::new().read(true).open(path)?;

    file.lock_shared()?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    file.unlock()?;

    Ok(contents)
}

/// Reads a JSON file under a shared lock and deserialises it into `T`.
///
/// # Errors
///
/// Returns an error if the file does not exist, cannot be read, or contains
/// invalid JSON.
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    if !path.exists() {
        return Err(format!("Config file not found: {}", path.display()).into());
    }
    let contents = read_locked(path)?;
    let value: T = serde_json::from_str(&contents)?;
    Ok(value)
}

/// Serialises `value` as pretty-printed JSON and writes it atomically to `path`.
fn write_json<T: serde::Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    let content = serde_json::to_string_pretty(value)?;
    atomic_write(path, &content)
}

/// Returns the path to `providers.json` inside the config directory.
pub fn providers_path() -> PathBuf {
    config_file("providers.json")
}

/// Returns the path to `groups.json` inside the config directory.
pub fn groups_path() -> PathBuf {
    config_file("groups.json")
}

/// Returns the path to `config.json` inside the config directory.
pub fn app_config_path() -> PathBuf {
    config_file("config.json")
}

/// Loads the list of providers from the on-disk JSON file.
///
/// # Errors
///
/// Returns an error if the file is missing or contains invalid JSON.
pub fn load_providers() -> Result<Vec<Provider>> {
    read_json(&providers_path())
}

/// Saves the list of providers to the on-disk JSON file atomically.
///
/// Creates the config directory if necessary.
pub fn save_providers(providers: &[Provider]) -> Result<()> {
    ensure_config_dir()?;
    write_json(&providers_path(), providers)
}

/// Atomically reads, modifies, and writes the providers list under an
/// exclusive file lock.
///
/// The closure `f` receives a mutable reference to the current providers
/// vector (which may be empty if the file did not previously exist). After
/// `f` returns the modified list is serialised back to disk in-place.
///
/// # Errors
///
/// Returns an error if the file cannot be opened, read, serialised, or
/// written.
pub fn update_providers_with_lock<F: FnOnce(&mut Vec<Provider>)>(f: F) -> Result<()> {
    let path = providers_path();
    ensure_config_dir()?;

    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&path)?;

    file.lock_exclusive()?;

    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    // An empty file means no providers yet — start with a blank vector
    let mut providers: Vec<Provider> = if contents.is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&contents)?
    };

    f(&mut providers);

    let content = serde_json::to_string_pretty(&providers)?;
    file.set_len(0)?;
    file.seek(std::io::SeekFrom::Start(0))?;
    file.write_all(content.as_bytes())?;
    file.flush()?;
    file.sync_all()?;

    // Re-assert 0600 permissions after the in-place rewrite
    let mut perms = file.metadata()?.permissions();
    perms.set_mode(0o600);
    file.set_permissions(perms)?;

    file.unlock()?;

    Ok(())
}

/// Loads the list of groups from the on-disk JSON file.
///
/// # Errors
///
/// Returns an error if the file is missing or contains invalid JSON.
pub fn load_groups() -> Result<Vec<Group>> {
    read_json(&groups_path())
}

/// Saves the list of groups to the on-disk JSON file atomically.
///
/// Creates the config directory if necessary.
pub fn save_groups(groups: &[Group]) -> Result<()> {
    ensure_config_dir()?;
    write_json(&groups_path(), groups)
}

/// Loads the application configuration from the on-disk JSON file.
///
/// # Errors
///
/// Returns an error if the file is missing or contains invalid JSON.
pub fn load_app_config() -> Result<AppConfig> {
    read_json(&app_config_path())
}

/// Saves the application configuration to the on-disk JSON file atomically.
///
/// Creates the config directory if necessary.
pub fn save_app_config(config: &AppConfig) -> Result<()> {
    ensure_config_dir()?;
    write_json(&app_config_path(), config)
}

/// Resets all configuration files to their defaults.
///
/// Writes `config.json`, `providers.json`, and `groups.json` with empty or
/// default values.
pub fn reset_all_config() -> Result<()> {
    ensure_config_dir()?;
    let default_config = AppConfig::default();
    write_json(&app_config_path(), &default_config)?;
    write_json(&providers_path(), &Vec::<Provider>::new())?;
    write_json(&groups_path(), &Vec::<Group>::new())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use crate::config::models::{
        AppConfig, FailoverConfig, Group, GroupEntry, Provider, ProviderModel,
    };

    static TEST_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn test_dir() -> PathBuf {
        let counter = TEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "coderouter_test_{}_{}_{}",
            std::process::id(),
            counter,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        path
    }

    fn setup_test_config_dir() -> PathBuf {
        let dir = test_dir();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup_test_config_dir(dir: &Path) {
        if dir.exists() {
            fs::remove_dir_all(dir).ok();
        }
    }

    fn test_provider() -> Provider {
        Provider {
            id: "test-provider".to_string(),
            name: "Test Provider".to_string(),
            protocol: "openai".to_string(),
            base_url: "https://api.test.com/v1".to_string(),
            credential_key: "test-provider".to_string(),
            daily_token_quota: Some(1_000_000),
            daily_request_quota: None,
            quota_reset_utc_hour: 0,
            enabled: true,
            models: vec![ProviderModel {
                id: "test-model".to_string(),
                context_window: Some(128000),
                max_output_tokens: Some(8192),
                input_cost_per_1m: Some(1.0),
                output_cost_per_1m: Some(2.0),
                last_refreshed: Some("2026-04-07T00:00:00Z".to_string()),
                protocol: None,
            }],
            model_overrides: None,
        }
    }

    fn test_group() -> Group {
        Group {
            id: "test-router".to_string(),
            alias: "test-router".to_string(),
            display_name: "Test Router".to_string(),
            entries: vec![GroupEntry {
                provider_id: "test-provider".to_string(),
                model_id: "test-model".to_string(),
                priority: 1,
                daily_token_quota_override: None,
                enabled: true,
                status: "active".to_string(),
                cooldown_until: None,
            }],
            failover_config: FailoverConfig {
                on_429: true,
                on_quota_exhausted: true,
                on_consecutive_errors: true,
                consecutive_error_threshold: 5,
                on_latency_timeout: true,
                latency_timeout_ms: 30000,
                latency_timeout_cooldown_ms: 300000,
                consecutive_error_cooldown_ms: 600000,
            },
        }
    }

    #[test]
    fn test_write_and_read_providers() {
        let test_dir = setup_test_config_dir();

        let providers = vec![test_provider()];
        let providers_path = test_dir.join("providers.json");

        write_json(&providers_path, &providers).unwrap();
        let loaded = read_json::<Vec<Provider>>(&providers_path).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "test-provider");
        assert_eq!(loaded[0].name, "Test Provider");
        assert_eq!(loaded[0].models.len(), 1);
        assert_eq!(loaded[0].models[0].id, "test-model");

        cleanup_test_config_dir(&test_dir);
    }

    #[test]
    fn test_write_and_read_groups() {
        let test_dir = setup_test_config_dir();

        let groups = vec![test_group()];
        let groups_path = test_dir.join("groups.json");

        write_json(&groups_path, &groups).unwrap();
        let loaded = read_json::<Vec<Group>>(&groups_path).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "test-router");
        assert_eq!(loaded[0].entries.len(), 1);
        assert_eq!(loaded[0].entries[0].priority, 1);
        assert!(loaded[0].failover_config.on_429);

        cleanup_test_config_dir(&test_dir);
    }

    #[test]
    fn test_write_and_read_app_config() {
        let test_dir = setup_test_config_dir();

        let config = AppConfig {
            proxy_port: 8080,
            proxy_host: "0.0.0.0".to_string(),
            refresh_interval_hours: 12,
            log_verbosity: "Debug".to_string(),
            opencode_config_path: None,
            onboarding_dismissed: false,
        };
        let config_path = test_dir.join("config.json");

        write_json(&config_path, &config).unwrap();
        let loaded = read_json::<AppConfig>(&config_path).unwrap();

        assert_eq!(loaded.proxy_port, 8080);
        assert_eq!(loaded.proxy_host, "0.0.0.0");
        assert_eq!(loaded.refresh_interval_hours, 12);

        cleanup_test_config_dir(&test_dir);
    }

    #[test]
    fn test_app_config_defaults() {
        let config = AppConfig::default();
        assert_eq!(config.proxy_port, 4141);
        assert_eq!(config.proxy_host, "127.0.0.1");
        assert_eq!(config.refresh_interval_hours, 24);
    }

    #[test]
    fn test_atomic_write_creates_file() {
        let test_dir = setup_test_config_dir();
        let file_path = test_dir.join("test.json");

        atomic_write(&file_path, r#"{"test": true}"#).unwrap();
        assert!(file_path.exists());

        let contents = fs::read_to_string(&file_path).unwrap();
        assert_eq!(contents, r#"{"test": true}"#);

        cleanup_test_config_dir(&test_dir);
    }

    #[test]
    fn test_missing_file_returns_error() {
        let test_dir = setup_test_config_dir();
        let file_path = test_dir.join("nonexistent.json");

        let result = read_json::<Vec<Provider>>(&file_path);
        assert!(result.is_err());

        cleanup_test_config_dir(&test_dir);
    }
}
