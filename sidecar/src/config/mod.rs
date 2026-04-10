//! Configuration module.
//!
//! Provides the serialisable configuration data models ([`models`]) and the
//! atomic JSON file store ([`store`]) used to persist and read configuration
//! from disk.

/// Serialisable configuration structs: providers, groups, failover, and app settings.
pub mod models;

/// Atomic, file-locked JSON read/write helpers for configuration files.
pub mod store;
