use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::models::{Group, GroupEntry, Provider};

static GLOBAL_ROUTER_STATE: std::sync::OnceLock<SharedRouterState> = std::sync::OnceLock::new();

pub fn get_global_router_state() -> Option<SharedRouterState> {
    GLOBAL_ROUTER_STATE.get().cloned()
}

pub fn set_global_router_state(state: SharedRouterState) {
    let _ = GLOBAL_ROUTER_STATE.set(state);
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub enum EntryStatus {
    Active,
    Cooldown,
    ManuallyDisabled,
    QuotaExhausted,
}

#[derive(Clone, Debug)]
pub struct EntryState {
    pub status: EntryStatus,
    pub cooldown_until: Option<DateTime<Utc>>,
    pub consecutive_errors: u32,
    pub daily_tokens_used: u64,
    pub daily_reset_at: DateTime<Utc>,
    pub cooldown_duration_seconds: Option<i64>,
}

impl EntryState {
    fn new(provider: &Provider) -> Self {
        let now = Utc::now();
        let reset_hour = provider.quota_reset_utc_hour.min(23);
        let mut daily_reset_at = now
            .date_naive()
            .and_hms_opt(reset_hour, 0, 0)
            .unwrap_or_else(|| now.date_naive().and_hms_opt(0, 0, 0).unwrap())
            .and_local_timezone(Utc)
            .single()
            .unwrap_or(now);
        if daily_reset_at <= now {
            daily_reset_at = daily_reset_at + chrono::Duration::days(1);
        }
        Self {
            status: EntryStatus::Active,
            cooldown_until: None,
            consecutive_errors: 0,
            daily_tokens_used: 0,
            daily_reset_at,
            cooldown_duration_seconds: Some(60),
        }
    }
}

#[derive(Default)]
pub struct RouterState {
    pub entries: HashMap<String, EntryState>,
}

pub type SharedRouterState = Arc<Mutex<RouterState>>;

pub fn entry_key(provider_id: &str, entry_index: u32) -> String {
    format!("{provider_id}:{entry_index}")
}

pub fn init_router_state(groups: &[Group], providers: &[Provider]) -> SharedRouterState {
    let mut state = RouterState::default();
    for group in groups {
        for (idx, entry) in group.entries.iter().enumerate() {
            if let Some(provider) = providers.iter().find(|p| p.id == entry.provider_id) {
                let key = entry_key(&entry.provider_id, idx as u32);
                let mut entry_state = EntryState::new(provider);
                if !entry.enabled {
                    entry_state.status = EntryStatus::ManuallyDisabled;
                }
                state.entries.insert(key, entry_state);
            }
        }
    }
    Arc::new(Mutex::new(state))
}

pub fn init_and_set_global_router_state(
    groups: &[Group],
    providers: &[Provider],
) -> SharedRouterState {
    let state = init_router_state(groups, providers);
    set_global_router_state(state.clone());
    state
}

pub fn init_daily_totals_from_db(state: &SharedRouterState, providers: &[Provider]) {
    use crate::metrics::db::init_db;
    use crate::metrics::queries::get_today_token_totals;

    let conn = match init_db() {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return,
    };

    for provider in providers {
        let totals = match get_today_token_totals(&conn, provider.quota_reset_utc_hour) {
            Ok(t) => t,
            Err(_) => continue,
        };

        for (provider_id, tokens) in totals {
            if provider_id == provider.id {
                for (key, entry_state) in guard.entries.iter_mut() {
                    if key.starts_with(&format!("{provider_id}:")) {
                        entry_state.daily_tokens_used = tokens;
                    }
                }
            }
        }
    }
}

pub fn select_entry<'a>(
    group: &'a Group,
    state: &RouterState,
    providers: &[Provider],
    skip_indices: &std::collections::HashSet<u32>,
) -> Option<(&'a GroupEntry, u32)> {
    let mut candidates: Vec<(&GroupEntry, u32)> = group
        .entries
        .iter()
        .enumerate()
        .filter(|(idx, entry)| {
            if skip_indices.contains(&(*idx as u32)) {
                return false;
            }
            if !entry.enabled {
                return false;
            }
            let key = entry_key(&entry.provider_id, *idx as u32);
            if let Some(entry_state) = state.entries.get(&key) {
                if entry_state.status != EntryStatus::Active {
                    if entry_state.status == EntryStatus::Cooldown {
                        if let Some(cooldown_until) = entry_state.cooldown_until {
                            if Utc::now() >= cooldown_until {
                                return true;
                            }
                        }
                    }
                    return false;
                }
                let effective_quota = entry.daily_token_quota_override.or_else(|| {
                    providers
                        .iter()
                        .find(|p| p.id == entry.provider_id)
                        .and_then(|p| p.daily_token_quota)
                });
                if let Some(quota) = effective_quota {
                    if entry_state.daily_tokens_used >= quota {
                        return false;
                    }
                }
                true
            } else {
                true
            }
        })
        .map(|(idx, entry)| (entry, idx as u32))
        .collect();

    candidates.sort_by_key(|(_, idx)| group.entries[*idx as usize].priority);

    candidates.into_iter().next()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EntryStatusResponse {
    pub group_id: String,
    pub group_alias: String,
    pub provider_id: String,
    pub model_id: String,
    pub priority: u32,
    pub entry_index: u32,
    pub status: String,
    pub cooldown_until: Option<DateTime<Utc>>,
    pub consecutive_errors: u32,
    pub daily_tokens_used: u64,
    pub daily_reset_at: DateTime<Utc>,
    pub cooldown_duration_seconds: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RouterStatusResponse {
    pub entries: Vec<EntryStatusResponse>,
}

pub fn get_router_status(groups: &[Group], state: &RouterState) -> RouterStatusResponse {
    let mut entries = Vec::new();
    for group in groups {
        for (idx, entry) in group.entries.iter().enumerate() {
            let key = entry_key(&entry.provider_id, idx as u32);
            let entry_state = state
                .entries
                .get(&key)
                .cloned()
                .unwrap_or_else(|| EntryState {
                    status: EntryStatus::Active,
                    cooldown_until: None,
                    consecutive_errors: 0,
                    daily_tokens_used: 0,
                    daily_reset_at: Utc::now(),
                    cooldown_duration_seconds: Some(60),
                });
            entries.push(EntryStatusResponse {
                group_id: group.id.clone(),
                group_alias: group.alias.clone(),
                provider_id: entry.provider_id.clone(),
                model_id: entry.model_id.clone(),
                priority: entry.priority,
                entry_index: idx as u32,
                status: format!("{:?}", entry_state.status).to_lowercase(),
                cooldown_until: entry_state.cooldown_until,
                consecutive_errors: entry_state.consecutive_errors,
                daily_tokens_used: entry_state.daily_tokens_used,
                daily_reset_at: entry_state.daily_reset_at,
                cooldown_duration_seconds: entry_state.cooldown_duration_seconds,
            });
        }
    }
    RouterStatusResponse { entries }
}

pub fn record_success(
    state: &mut RouterState,
    provider_id: &str,
    entry_index: u32,
    tokens_used: u64,
) {
    let key = entry_key(provider_id, entry_index);
    if let Some(entry_state) = state.entries.get_mut(&key) {
        entry_state.consecutive_errors = 0;
        entry_state.daily_tokens_used += tokens_used;
        if entry_state.status == EntryStatus::Cooldown {
            entry_state.status = EntryStatus::Active;
            entry_state.cooldown_until = None;
        }
    }
}

pub fn record_429(
    state: &mut RouterState,
    provider_id: &str,
    entry_index: u32,
    base_backoff_seconds: i64,
) {
    let key = entry_key(provider_id, entry_index);
    if let Some(entry_state) = state.entries.get_mut(&key) {
        let was_in_cooldown = entry_state.status == EntryStatus::Cooldown;
        let current_backoff = entry_state
            .cooldown_duration_seconds
            .unwrap_or(base_backoff_seconds);
        let new_backoff = if !was_in_cooldown {
            base_backoff_seconds
        } else {
            (current_backoff * 2).min(3600).max(base_backoff_seconds)
        };
        entry_state.cooldown_duration_seconds = Some(new_backoff);
        entry_state.status = EntryStatus::Cooldown;
        entry_state.cooldown_until = Some(Utc::now() + chrono::Duration::seconds(new_backoff));
    }
}

pub fn record_quota_exhausted(state: &mut RouterState, provider_id: &str, entry_index: u32) {
    let key = entry_key(provider_id, entry_index);
    if let Some(entry_state) = state.entries.get_mut(&key) {
        entry_state.status = EntryStatus::QuotaExhausted;
        entry_state.cooldown_until = None;
    }
}

pub fn record_consecutive_error(
    state: &mut RouterState,
    provider_id: &str,
    entry_index: u32,
    threshold: u32,
    trigger_enabled: bool,
) {
    let key = entry_key(provider_id, entry_index);
    if let Some(entry_state) = state.entries.get_mut(&key) {
        entry_state.consecutive_errors += 1;
        if trigger_enabled && entry_state.consecutive_errors >= threshold {
            entry_state.status = EntryStatus::Cooldown;
            entry_state.cooldown_until = Some(Utc::now() + chrono::Duration::minutes(10));
            entry_state.consecutive_errors = 0;
        }
    }
}

pub fn record_latency_timeout(
    state: &mut RouterState,
    provider_id: &str,
    entry_index: u32,
) -> Result<(), &'static str> {
    let key = entry_key(provider_id, entry_index);
    if let Some(entry_state) = state.entries.get_mut(&key) {
        entry_state.status = EntryStatus::Cooldown;
        entry_state.cooldown_until = Some(Utc::now() + chrono::Duration::minutes(5));
        Ok(())
    } else {
        Err("entry state not found")
    }
}

pub fn build_exhausted_response(alias: &str) -> serde_json::Value {
    serde_json::json!({
        "error": {
            "message": format!("All providers for model '{alias}' are currently unavailable."),
            "type": "coderouter_error",
            "code": "all_providers_exhausted"
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_provider(id: &str, quota: Option<u64>) -> Provider {
        Provider {
            id: id.to_string(),
            name: id.to_string(),
            protocol: "openai".to_string(),
            base_url: "https://api.test.com/v1".to_string(),
            credential_key: id.to_string(),
            daily_token_quota: quota,
            quota_reset_utc_hour: 0,
            enabled: true,
            models: vec![],
            model_overrides: None,
        }
    }

    fn test_group_with_entries(entries: Vec<GroupEntry>) -> Group {
        Group {
            id: "test-group".to_string(),
            alias: "test-group".to_string(),
            display_name: "Test Group".to_string(),
            entries,
            failover_config: crate::config::models::FailoverConfig {
                on_429: true,
                on_quota_exhausted: true,
                on_consecutive_errors: true,
                consecutive_error_threshold: 3,
                on_latency_timeout: true,
                latency_timeout_ms: 30000,
            },
        }
    }

    fn make_entry(provider_id: &str, priority: u32, enabled: bool) -> GroupEntry {
        GroupEntry {
            provider_id: provider_id.to_string(),
            model_id: "test-model".to_string(),
            priority,
            daily_token_quota_override: None,
            enabled,
            status: "active".to_string(),
            cooldown_until: None,
        }
    }

    #[test]
    fn test_select_entry_returns_lowest_priority() {
        let providers = vec![test_provider("p1", None), test_provider("p2", None)];
        let entries = vec![make_entry("p1", 2, true), make_entry("p2", 1, true)];
        let group = test_group_with_entries(entries);
        let state = init_router_state(&[group.clone()], &providers);
        let state = state.lock().unwrap();
        let skip = std::collections::HashSet::new();

        let (entry, idx) = select_entry(&group, &state, &providers, &skip).unwrap();
        assert_eq!(idx, 1);
        assert_eq!(entry.provider_id, "p2");
    }

    #[test]
    fn test_select_entry_skips_disabled() {
        let providers = vec![test_provider("p1", None)];
        let entries = vec![make_entry("p1", 1, false)];
        let group = test_group_with_entries(entries);
        let state = init_router_state(&[group.clone()], &providers);
        let state = state.lock().unwrap();
        let skip = std::collections::HashSet::new();

        assert!(select_entry(&group, &state, &providers, &skip).is_none());
    }

    #[test]
    fn test_select_entry_skips_cooldown() {
        let providers = vec![test_provider("p1", None)];
        let entries = vec![make_entry("p1", 1, true)];
        let group = test_group_with_entries(entries);
        let state = init_router_state(&[group.clone()], &providers);
        {
            let mut s = state.lock().unwrap();
            record_429(&mut s, "p1", 0, 60);
        }
        let state = state.lock().unwrap();
        let skip = std::collections::HashSet::new();

        assert!(select_entry(&group, &state, &providers, &skip).is_none());
    }

    #[test]
    fn test_select_entry_skips_quota_exhausted() {
        let providers = vec![test_provider("p1", Some(100))];
        let entries = vec![make_entry("p1", 1, true)];
        let group = test_group_with_entries(entries);
        let state = init_router_state(&[group.clone()], &providers);
        {
            let mut s = state.lock().unwrap();
            record_success(&mut s, "p1", 0, 100);
        }
        let state = state.lock().unwrap();
        let skip = std::collections::HashSet::new();

        assert!(select_entry(&group, &state, &providers, &skip).is_none());
    }

    #[test]
    fn test_record_success_resets_errors() {
        let providers = vec![test_provider("p1", None)];
        let entries = vec![make_entry("p1", 1, true)];
        let group = test_group_with_entries(entries);
        let state = init_router_state(&[group.clone()], &providers);
        {
            let mut s = state.lock().unwrap();
            record_consecutive_error(&mut s, "p1", 0, 3, true);
            record_consecutive_error(&mut s, "p1", 0, 3, true);
            record_success(&mut s, "p1", 0, 50);
        }
        let state = state.lock().unwrap();
        let entry = state.entries.get("p1:0").unwrap();
        assert_eq!(entry.consecutive_errors, 0);
        assert_eq!(entry.daily_tokens_used, 50);
    }

    #[test]
    fn test_record_429_sets_cooldown() {
        let providers = vec![test_provider("p1", None)];
        let entries = vec![make_entry("p1", 1, true)];
        let group = test_group_with_entries(entries);
        let state = init_router_state(&[group.clone()], &providers);
        {
            let mut s = state.lock().unwrap();
            record_429(&mut s, "p1", 0, 60);
        }
        let state = state.lock().unwrap();
        let entry = state.entries.get("p1:0").unwrap();
        assert_eq!(entry.status, EntryStatus::Cooldown);
        assert!(entry.cooldown_until.is_some());
    }

    #[test]
    fn test_record_consecutive_error_triggers_cooldown() {
        let providers = vec![test_provider("p1", None)];
        let entries = vec![make_entry("p1", 1, true)];
        let group = test_group_with_entries(entries);
        let state = init_router_state(&[group.clone()], &providers);
        {
            let mut s = state.lock().unwrap();
            record_consecutive_error(&mut s, "p1", 0, 3, true);
            record_consecutive_error(&mut s, "p1", 0, 3, true);
            record_consecutive_error(&mut s, "p1", 0, 3, true);
        }
        let state = state.lock().unwrap();
        let entry = state.entries.get("p1:0").unwrap();
        assert_eq!(entry.status, EntryStatus::Cooldown);
        assert_eq!(entry.consecutive_errors, 0);
    }

    #[test]
    fn test_record_quota_exhausted() {
        let providers = vec![test_provider("p1", None)];
        let entries = vec![make_entry("p1", 1, true)];
        let group = test_group_with_entries(entries);
        let state = init_router_state(&[group.clone()], &providers);
        {
            let mut s = state.lock().unwrap();
            record_quota_exhausted(&mut s, "p1", 0);
        }
        let state = state.lock().unwrap();
        let entry = state.entries.get("p1:0").unwrap();
        assert_eq!(entry.status, EntryStatus::QuotaExhausted);
    }

    #[test]
    fn test_record_latency_timeout() {
        let providers = vec![test_provider("p1", None)];
        let entries = vec![make_entry("p1", 1, true)];
        let group = test_group_with_entries(entries);
        let state = init_router_state(&[group.clone()], &providers);
        {
            let mut s = state.lock().unwrap();
            let result = record_latency_timeout(&mut s, "p1", 0);
            assert!(result.is_ok());
        }
        let state = state.lock().unwrap();
        let entry = state.entries.get("p1:0").unwrap();
        assert_eq!(entry.status, EntryStatus::Cooldown);
    }

    #[test]
    fn test_build_exhausted_response() {
        let resp = build_exhausted_response("my-model");
        assert_eq!(
            resp["error"]["message"],
            "All providers for model 'my-model' are currently unavailable."
        );
        assert_eq!(resp["error"]["type"], "coderouter_error");
        assert_eq!(resp["error"]["code"], "all_providers_exhausted");
    }

    #[test]
    fn test_select_entry_with_skip_indices() {
        let providers = vec![test_provider("p1", None), test_provider("p2", None)];
        let entries = vec![make_entry("p1", 1, true), make_entry("p2", 2, true)];
        let group = test_group_with_entries(entries);
        let state = init_router_state(&[group.clone()], &providers);
        let state = state.lock().unwrap();
        let mut skip = std::collections::HashSet::new();
        skip.insert(0);

        let (entry, idx) = select_entry(&group, &state, &providers, &skip).unwrap();
        assert_eq!(idx, 1);
        assert_eq!(entry.provider_id, "p2");
    }

    #[test]
    fn test_get_router_status() {
        let providers = vec![test_provider("p1", None)];
        let entries = vec![make_entry("p1", 1, true)];
        let group = test_group_with_entries(entries);
        let state = init_router_state(&[group.clone()], &providers);
        let state = state.lock().unwrap();

        let status = get_router_status(&[group.clone()], &state);
        assert_eq!(status.entries.len(), 1);
        assert_eq!(status.entries[0].group_alias, "test-group");
        assert_eq!(status.entries[0].provider_id, "p1");
        assert_eq!(status.entries[0].status, "active");
    }

    #[test]
    fn test_record_success_transitions_cooldown_to_active() {
        let providers = vec![test_provider("p1", None)];
        let entries = vec![make_entry("p1", 1, true)];
        let group = test_group_with_entries(entries);
        let state = init_router_state(&[group.clone()], &providers);
        {
            let mut s = state.lock().unwrap();
            record_429(&mut s, "p1", 0, 60);
            let entry = s.entries.get("p1:0").unwrap();
            assert_eq!(entry.status, EntryStatus::Cooldown);
            assert!(entry.cooldown_until.is_some());
            record_success(&mut s, "p1", 0, 50);
            let entry = s.entries.get("p1:0").unwrap();
            assert_eq!(entry.status, EntryStatus::Active);
            assert!(entry.cooldown_until.is_none());
        }
    }

    #[test]
    fn test_record_429_first_backoff_starts_at_base() {
        let providers = vec![test_provider("p1", None)];
        let entries = vec![make_entry("p1", 1, true)];
        let group = test_group_with_entries(entries);
        let state = init_router_state(&[group.clone()], &providers);
        {
            let mut s = state.lock().unwrap();
            record_429(&mut s, "p1", 0, 60);
            let entry = s.entries.get("p1:0").unwrap();
            assert_eq!(entry.cooldown_duration_seconds, Some(60));
        }
    }

    #[test]
    fn test_record_429_subsequent_backoff_doubles() {
        let providers = vec![test_provider("p1", None)];
        let entries = vec![make_entry("p1", 1, true)];
        let group = test_group_with_entries(entries);
        let state = init_router_state(&[group.clone()], &providers);
        {
            let mut s = state.lock().unwrap();
            record_429(&mut s, "p1", 0, 60);
            record_429(&mut s, "p1", 0, 60);
            let entry = s.entries.get("p1:0").unwrap();
            assert_eq!(entry.cooldown_duration_seconds, Some(120));
        }
    }

    #[test]
    fn test_record_quota_exhausted_clears_cooldown_until() {
        let providers = vec![test_provider("p1", None)];
        let entries = vec![make_entry("p1", 1, true)];
        let group = test_group_with_entries(entries);
        let state = init_router_state(&[group.clone()], &providers);
        {
            let mut s = state.lock().unwrap();
            record_429(&mut s, "p1", 0, 60);
            let entry = s.entries.get("p1:0").unwrap();
            assert!(entry.cooldown_until.is_some());
            record_quota_exhausted(&mut s, "p1", 0);
            let entry = s.entries.get("p1:0").unwrap();
            assert_eq!(entry.status, EntryStatus::QuotaExhausted);
            assert!(entry.cooldown_until.is_none());
        }
    }
}
