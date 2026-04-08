use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use reqwest::Client;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};

use crate::config::models::{Group, Provider};
use crate::config::store;
use crate::credentials::keychain;
use crate::proxy::router::{
    self, EntryStatus, SharedRouterState,
};

const BASE_COOLDOWN_SECONDS: i64 = 60;
const MAX_COOLDOWN_SECONDS: i64 = 3600;

pub struct ProbeLock {
    in_flight: Mutex<HashSet<String>>,
}

impl ProbeLock {
    pub fn new() -> Self {
        Self {
            in_flight: Mutex::new(HashSet::new()),
        }
    }

    pub async fn try_acquire(&self, key: &str) -> Option<ProbeGuard<'_>> {
        let mut set = self.in_flight.lock().await;
        if set.insert(key.to_string()) {
            Some(ProbeGuard {
                lock: self,
                key: key.to_string(),
            })
        } else {
            None
        }
    }
}

pub struct ProbeGuard<'a> {
    lock: &'a ProbeLock,
    key: String,
}

impl Drop for ProbeGuard<'_> {
    fn drop(&mut self) {
        let _ = self.lock.in_flight.try_lock().map(|mut set| {
            set.remove(&self.key);
        });
    }
}

pub fn spawn_scheduler(
    router_state: SharedRouterState,
    groups: Arc<Vec<Group>>,
    client: Client,
) -> tokio::task::JoinHandle<()> {
    let state_clone = router_state.clone();
    let groups_clone = groups.clone();
    let client_clone = client.clone();

    tokio::spawn(async move {
        let probe_lock = Arc::new(ProbeLock::new());
        let mut quota_interval = interval(Duration::from_secs(60));
        let mut cooldown_interval = interval(Duration::from_secs(30));

        loop {
            tokio::select! {
                _ = quota_interval.tick() => {
                    run_quota_reset(&state_clone, &groups_clone);
                }
                _ = cooldown_interval.tick() => {
                    run_cooldown_check(&state_clone, &groups_clone, &client_clone, &probe_lock).await;
                }
            }
        }
    })
}

fn run_quota_reset(state: &SharedRouterState, groups: &[Group]) {
    let now = Utc::now();
    let providers = match store::load_providers() {
        Ok(p) => p,
        Err(_) => return,
    };

    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return,
    };

    for group in groups {
        for (idx, entry) in group.entries.iter().enumerate() {
            let key = router::entry_key(&entry.provider_id, idx as u32);
            let entry_state = match guard.entries.get_mut(&key) {
                Some(s) if s.status == EntryStatus::QuotaExhausted => s,
                _ => continue,
            };

            let provider = match providers.iter().find(|p| p.id == entry.provider_id) {
                Some(p) => p,
                None => continue,
            };

            let reset_hour = provider.quota_reset_utc_hour;
            let today_reset = now
                .date_naive()
                .and_hms_opt(reset_hour, 0, 0)
                .unwrap()
                .and_local_timezone(Utc)
                .single();

            let should_reset = match today_reset {
                Some(t) => now >= t,
                None => now >= now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_local_timezone(Utc).single().unwrap(),
            };

            if should_reset {
                entry_state.status = EntryStatus::Active;
                entry_state.daily_tokens_used = 0;
                let next_reset = now
                    .date_naive()
                    .and_hms_opt(reset_hour, 0, 0)
                    .unwrap()
                    .and_local_timezone(Utc)
                    .single()
                    .unwrap_or(now)
                    + chrono::Duration::days(1);
                entry_state.daily_reset_at = next_reset;
            }
        }
    }
}

async fn run_cooldown_check(
    state: &SharedRouterState,
    groups: &[Group],
    client: &Client,
    probe_lock: &Arc<ProbeLock>,
) {
    let now = Utc::now();
    let providers = match store::load_providers() {
        Ok(p) => p,
        Err(_) => return,
    };

    let mut probes = Vec::new();

    {
        let guard = match state.lock() {
            Ok(g) => g,
            Err(_) => return,
        };

        for group in groups {
            for (idx, entry) in group.entries.iter().enumerate() {
                let key = router::entry_key(&entry.provider_id, idx as u32);
                let entry_state = match guard.entries.get(&key) {
                    Some(s) if s.status == EntryStatus::Cooldown => s,
                    _ => continue,
                };

                let _cooldown_until = match entry_state.cooldown_until {
                    Some(t) if now >= t => t,
                    _ => continue,
                };

                let provider = match providers.iter().find(|p| p.id == entry.provider_id) {
                    Some(p) => p.clone(),
                    None => continue,
                };

                let cooldown_duration = entry_state.cooldown_duration_seconds.unwrap_or(BASE_COOLDOWN_SECONDS);
                probes.push((key.clone(), provider, cooldown_duration));
            }
        }
    }

    for (key, provider, cooldown_duration) in probes {
        let lock = probe_lock.clone();
        let state = state.clone();
        let client = client.clone();

        if lock.try_acquire(&key).await.is_none() {
            continue;
        }

        tokio::spawn(async move {
            let result = run_probe(&provider, &client).await;
            handle_probe_result(&state, &key, result, cooldown_duration);
        });
    }
}

async fn run_probe(provider: &Provider, client: &Client) -> ProbeResult {
    let api_key = match keychain::get_credential(&provider.credential_key).await {
        Ok(k) => k,
        Err(e) => return ProbeResult::Error(format!("credential error: {e}")),
    };

    let models_url = format!("{}/v1/models", provider.base_url.trim_end_matches('/'));

    let req = match provider.protocol.as_str() {
        "anthropic" => client
            .get(&models_url)
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01"),
        _ => client
            .get(&models_url)
            .header("Authorization", format!("Bearer {api_key}")),
    };

    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            if status == 429 {
                ProbeResult::RateLimited
            } else if resp.status().is_success() {
                ProbeResult::Success
            } else {
                let body = resp.text().await.unwrap_or_default();
                ProbeResult::Error(format!("HTTP {status}: {body}"))
            }
        }
        Err(e) => ProbeResult::Error(format!("network error: {e}")),
    }
}

enum ProbeResult {
    Success,
    RateLimited,
    Error(String),
}

fn handle_probe_result(
    state: &SharedRouterState,
    key: &str,
    result: ProbeResult,
    current_cooldown: i64,
) {
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return,
    };

    let entry_state = match guard.entries.get_mut(key) {
        Some(s) => s,
        None => return,
    };

    match result {
        ProbeResult::Success => {
            entry_state.status = EntryStatus::Active;
            entry_state.cooldown_until = None;
            entry_state.consecutive_errors = 0;
            entry_state.cooldown_duration_seconds = Some(BASE_COOLDOWN_SECONDS);
        }
        ProbeResult::RateLimited => {
            let new_duration = (current_cooldown * 2).min(MAX_COOLDOWN_SECONDS);
            entry_state.cooldown_until = Some(Utc::now() + chrono::Duration::seconds(new_duration));
            entry_state.cooldown_duration_seconds = Some(new_duration);
        }
        ProbeResult::Error(_) => {
            let new_until = Utc::now() + chrono::Duration::seconds(current_cooldown);
            entry_state.cooldown_until = Some(new_until);
            entry_state.cooldown_duration_seconds = Some(current_cooldown);
        }
    }
}

pub fn set_entry_enabled(
    state: &SharedRouterState,
    groups: Arc<Vec<Group>>,
    group_id: &str,
    entry_index: usize,
    enabled: bool,
) -> Result<(), String> {
    let groups_ref = groups.as_ref();
    let group = groups_ref
        .iter()
        .find(|g| g.id == group_id)
        .ok_or_else(|| format!("Group '{group_id}' not found"))?;

    if entry_index >= group.entries.len() {
        return Err(format!(
            "Entry index {entry_index} out of range for group '{group_id}'"
        ));
    }

    {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        let key = router::entry_key(&group.entries[entry_index].provider_id, entry_index as u32);
        let entry_state = guard
            .entries
            .get_mut(&key)
            .ok_or_else(|| "Entry state not found".to_string())?;

        if enabled {
            entry_state.status = EntryStatus::Active;
            entry_state.cooldown_until = None;
            entry_state.consecutive_errors = 0;
            entry_state.cooldown_duration_seconds = Some(BASE_COOLDOWN_SECONDS);
        } else {
            entry_state.status = EntryStatus::ManuallyDisabled;
            entry_state.cooldown_until = None;
        }
    }

    if !enabled {
        let mut updated_groups = (*groups).clone();
        if let Some(g) = updated_groups.iter_mut().find(|g| g.id == group_id) {
            g.entries[entry_index].enabled = false;
        }
        store::save_groups(&updated_groups).map_err(|e| e.to_string())?;
    }

    Ok(())
}
