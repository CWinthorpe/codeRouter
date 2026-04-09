use secret_service::{EncryptionType, SecretService};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const SERVICE_NAME: &str = "coderouter";
const ATTRIBUTE_KEY: &str = "provider_id";

fn fallback_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("coderouter");
    path.push("credentials.json");
    path
}

fn read_fallback_file() -> HashMap<String, String> {
    let path = fallback_path();
    let data = match fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => return HashMap::new(),
    };
    serde_json::from_str(&data).unwrap_or_default()
}

fn write_fallback_file(map: &HashMap<String, String>) -> Result<()> {
    let path = fallback_path();
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    let json = serde_json::to_string_pretty(map)?;
    fs::write(&path, &json)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

async fn try_store_secret_service(provider_id: &str, api_key: &str) -> Result<()> {
    let ss = SecretService::connect(EncryptionType::Dh).await?;
    let collection = ss.get_default_collection().await?;

    let attributes = HashMap::from([(ATTRIBUTE_KEY, provider_id)]);

    collection
        .create_item(
            SERVICE_NAME,
            attributes,
            api_key.as_bytes(),
            true,
            "text/plain",
        )
        .await?;

    Ok(())
}

async fn try_get_secret_service(provider_id: &str) -> Result<String> {
    let ss = SecretService::connect(EncryptionType::Dh).await?;
    let collection = ss.get_default_collection().await?;

    let attributes = HashMap::from([(ATTRIBUTE_KEY, provider_id)]);

    let items = collection.search_items(attributes).await?;

    let item = items.first().ok_or("Credential not found")?;

    let secret = item.get_secret().await?;

    String::from_utf8(secret).map_err(|e| e.into())
}

async fn try_delete_secret_service(provider_id: &str) -> Result<()> {
    let ss = SecretService::connect(EncryptionType::Dh).await?;
    let collection = ss.get_default_collection().await?;

    let attributes = HashMap::from([(ATTRIBUTE_KEY, provider_id)]);

    let items = collection.search_items(attributes).await?;

    if let Some(item) = items.first() {
        item.delete().await?;
    }

    Ok(())
}

pub async fn store_credential(provider_id: &str, api_key: &str) -> Result<()> {
    match try_store_secret_service(provider_id, api_key).await {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("[credentials] Secret Service unavailable, using file fallback: {e}");
            let mut map = read_fallback_file();
            map.insert(provider_id.to_string(), api_key.to_string());
            write_fallback_file(&map)?;
            Ok(())
        }
    }
}

pub async fn get_credential(provider_id: &str) -> Result<String> {
    match try_get_secret_service(provider_id).await {
        Ok(val) => Ok(val),
        Err(e) => {
            eprintln!("[credentials] Secret Service unavailable, using file fallback: {e}");
            let map = read_fallback_file();
            map.get(provider_id)
                .cloned()
                .ok_or("Credential not found".into())
        }
    }
}

pub async fn delete_credential(provider_id: &str) -> Result<()> {
    match try_delete_secret_service(provider_id).await {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("[credentials] Secret Service unavailable, using file fallback: {e}");
            let mut map = read_fallback_file();
            map.remove(provider_id);
            write_fallback_file(&map)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_file_round_trip() {
        let test_id = "coderouter-test-fallback-credential";
        let path = fallback_path();

        let mut map = read_fallback_file();
        map.insert(test_id.to_string(), "test-key-123".to_string());
        write_fallback_file(&map).unwrap();

        let loaded = read_fallback_file();
        assert_eq!(loaded.get(test_id), Some(&"test-key-123".to_string()));

        map.remove(test_id);
        write_fallback_file(&map).unwrap();

        let meta = fs::metadata(&path).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[tokio::test]
    async fn test_store_get_delete_credential() {
        let test_id = "coderouter-test-provider-credential";

        let _ = delete_credential(test_id).await;

        store_credential(test_id, "test-api-key-12345").await.unwrap();

        let retrieved = get_credential(test_id).await.unwrap();
        assert_eq!(retrieved, "test-api-key-12345");

        delete_credential(test_id).await.unwrap();

        let result = get_credential(test_id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_nonexistent_credential_returns_error() {
        let result = get_credential("nonexistent-provider-that-does-not-exist").await;
        assert!(result.is_err());
    }
}
