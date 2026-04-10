//! coderouter-proxy sidecar library.
//!
//! This crate provides the core modules for the coderouter proxy sidecar,
//! including configuration management, credential storage, request metrics,
//! data models, opencode integration, and the proxy server itself.

/// Configuration loading, saving, and data models.
pub mod config;

/// Credential storage backed by the OS secret service with a file fallback.
pub mod credentials;

/// Request and usage metrics collection.
pub mod metrics;

/// Shared data structures used across the proxy.
pub mod models;

/// Integration with the opencode configuration.
pub mod opencode;

/// The HTTP proxy server and request routing logic.
pub mod proxy;
