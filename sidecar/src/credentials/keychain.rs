use secret_service::{EncryptionType, SecretService};
use std::collections::HashMap;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const SERVICE_NAME: &str = "coderouter";
const ATTRIBUTE_KEY: &str = "provider_id";

async fn get_service() -> Result<SecretService<'static>> {
    let ss = SecretService::connect(EncryptionType::Dh).await?;
    Ok(ss)
}

pub async fn store_credential(provider_id: &str, api_key: &str) -> Result<()> {
    let ss = get_service().await?;
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

pub async fn get_credential(provider_id: &str) -> Result<String> {
    let ss = get_service().await?;
    let collection = ss.get_default_collection().await?;

    let attributes = HashMap::from([(ATTRIBUTE_KEY, provider_id)]);

    let items = collection.search_items(attributes).await?;

    let item = items.first().ok_or("Credential not found")?;

    let secret = item.get_secret().await?;

    String::from_utf8(secret).map_err(|e| e.into())
}

pub async fn delete_credential(provider_id: &str) -> Result<()> {
    let ss = get_service().await?;
    let collection = ss.get_default_collection().await?;

    let attributes = HashMap::from([(ATTRIBUTE_KEY, provider_id)]);

    let items = collection.search_items(attributes).await?;

    if let Some(item) = items.first() {
        item.delete().await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
