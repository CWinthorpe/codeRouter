//! Proxy module: handles incoming HTTP requests, routes them to upstream
//! LLM providers, translates between OpenAI and Anthropic/Codex protocols,
//! and manages failover/quota/cooldown state.

pub mod codex;
pub mod router;
pub mod server;
pub mod ssrf;
pub mod translator;
pub mod upstream;
