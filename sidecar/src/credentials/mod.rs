//! Credential management module.
//!
//! Provides secure storage and retrieval of API keys, using the system
//! secret service (e.g. GNOME Keyring / KDE Wallet via libsecret) with an
//! encrypted file fallback when the secret service is unavailable.

/// Secret-service and file-fallback credential store.
pub mod keychain;
