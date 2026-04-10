//! Metrics module for tracking API request usage, costs, and latency.
//!
//! Provides persistent SQLite-based storage for request events, a non-blocking
//! channel-based recorder, query functions for dashboards, and a background
//! scheduler for quota resets and provider cooldown management.

pub mod db;
pub mod queries;
pub mod recorder;
pub mod scheduler;