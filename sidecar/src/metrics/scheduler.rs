use std::collections::HashSet;
use std::sync::Arc;
use std::sync::RwLock;

use chrono::Utc;
use reqwest::Client;
use tokio::sync::Mutex;
use tokio::sync::oneshot;
use tokio::time::{interval, Duration};

use crate::config::models::{Group, Provider};
use crate::config::store;
use crate::credentials::keychain;
use crate::proxy::router::{
    self, EntryStatus, SharedRouterState,
};

/// Starting cooldown duration in seconds, used when a provider first enters
/// cooldown and is reset when a probe succeeds.
const BASE_COOLDOWN_SECONDS: i64 = 60;

/// Maximum cooldown cap in seconds — exponential backoff will never exceed this.
const MAX_COOLDOWN_SECONDS: i64 = 3600;

/// A lock that prevents concurrent probes for the same provider entry.
///
/// Because probes are asynchronous and may race, we need to ensure that at most
/// one probe per entry is in flight at any time.
pub struct ProbeLock {
    in_flight: Mutex<HashSet<String>>,
}

impl ProbeLock {
    /// Creates a new, empty probe lock.
    pub fn new() -> Self {
        Self {
            in_flight: Mutex::new(HashSet::new()),
        }
    }

    /// Attempts to register a probe for the given key.
    ///
    /// Returns `Some(ProbeGuard)` if the key was not already in flight, or
    /// `None` if a probe is already running for that key. The guard
    /// automatically removes the key from the set when dropped.
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

/// RAII guard that holds ownership of a probe slot. Dropping the guard removes
/// the key from [`ProbeLock`], allowing a future probe to be scheduled.
pub struct ProbeGuard<'a> {
    lock: &'a ProbeLock,
    key: String,
}

impl Drop for ProbeGuard<'_> {
    fn drop(&mut self) {
        // Use blocking_lock because Drop cannot be async.
        let mut set = self.lock.in_flight.blocking_lock();
        set.remove(&self.key);
    }
}

/// Spawns a background scheduler that periodically resets daily quotas and
/// probes cooldown entries to check if they can be re-enabled.
///
/// The scheduler runs two tick loops:
/// - Quota reset every 60 seconds — resets token/request counters for entries
///   whose daily reset timestamp has passed.
/// - Cooldown check every 30 seconds — probes entries in cooldown whose expiry
///   time has passed; on success the entry is re-enabled, on 429 the cooldown
///   duration doubles (capped at [`MAX_COOLDOWN_SECONDS`]).
///
/// # Returns
///
/// A `(JoinHandle, oneshot::Sender)` pair. Sending a value on the sender
/// triggers a graceful shutdown.
pub fn spawn_scheduler(
    router_state: SharedRouterState,
    groups: Arc<RwLock<Arc<Vec<Group>>>>,
    client: Client,
) -> (tokio::task::JoinHandle<()>, oneshot::Sender<()>) {
    let state_clone = router_state.clone();
    let groups_clone = groups.clone();

    // Separate client with a short timeout so probes don't block the scheduler.
    let probe_client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| client.clone());

    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

    let handle = tokio::spawn(async move {
        let probe_lock = Arc::new(ProbeLock::new());
        let mut quota_interval = interval(Duration::from_secs(60));
        let mut cooldown_interval = interval(Duration::from_secs(30));

        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    eprintln!("[scheduler] Shutting down gracefully");
                    break;
                }
                _ = quota_interval.tick() => {
                    let groups_snapshot = {
                        let guard = groups_clone.read().unwrap();
                        guard.clone()
                    };
                    run_quota_reset(&state_clone, &groups_snapshot);
                }
                _ = cooldown_interval.tick() => {
                    let groups_snapshot = {
                        let guard = groups_clone.read().unwrap();
                        guard.clone()
                    };
                    run_cooldown_check(&state_clone, &groups_snapshot, &probe_client, &probe_lock).await;
                }
            }
        }
    });

    (handle, shutdown_tx)
}

/// Iterates all router entries and resets daily quota counters for entries
/// whose `daily_reset_at` timestamp has passed.
///
/// Skips entries that are [`EntryStatus::ManuallyDisabled`] — those should only
/// be re-enabled via explicit admin action.
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
                // Manually-disabled entries must not be auto-reset.
                Some(s) if s.status != EntryStatus::ManuallyDisabled => s,
                _ => continue,
            };

            let provider = match providers.iter().find(|p| p.id == entry.provider_id) {
                Some(p) => p,
                None => continue,
            };

            // Clamp to 0–23 to avoid invalid time-of-day values.
            let reset_hour = provider.quota_reset_utc_hour.min(23);

            if now >= entry_state.daily_reset_at {
                entry_state.status = EntryStatus::Active;
                entry_state.daily_tokens_used = 0;
                entry_state.daily_requests_used = 0;
                entry_state.consecutive_errors = 0;
                // Schedule the next reset for the start of the next provider-day.
                let next_reset = now
                    .date_naive()
                    .and_hms_opt(reset_hour, 0, 0)
                    .unwrap_or_else(|| now.date_naive().and_hms_opt(0, 0, 0).unwrap())
                    .and_local_timezone(Utc)
                    .single()
                    .unwrap_or(now)
                    + chrono::Duration::days(1);
                entry_state.daily_reset_at = next_reset;
            }
        }
    }
}

/// Checks all entries in [`EntryStatus::Cooldown`] whose cooldown has expired
/// and probes the upstream provider to decide whether to re-enable them.
///
/// Probes are deduplicated via [`ProbeLock`] so that at most one probe per
/// entry runs concurrently. Each probe runs in a separate tokio task.
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
                    // Only probe entries that are in cooldown.
                    Some(s) if s.status == EntryStatus::Cooldown => s,
                    _ => continue,
                };

                // Skip if the cooldown hasn't expired yet.
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

    // Drop the lock before spawning probe tasks so they can acquire it again.
    for (key, provider, cooldown_duration) in probes {
        let lock = probe_lock.clone();
        let state = state.clone();
        let client = client.clone();

        // Skip this entry if a probe is already in flight.
        if lock.try_acquire(&key).await.is_none() {
            continue;
        }

        tokio::spawn(async move {
            let result = run_probe(&provider, &client).await;
            handle_probe_result(&state, &key, result, cooldown_duration);
        });
    }
}

/// Probes a single provider by calling its `/v1/models` endpoint.
///
/// Uses the provider's credential key and adjusts the request headers
/// depending on whether the provider uses the Anthropic protocol.
async fn run_probe(provider: &Provider, client: &Client) -> ProbeResult {
    let api_key = match keychain::get_credential(&provider.credential_key).await {
        Ok(k) => k,
        Err(_) => return ProbeResult::Error,
    };

    let models_url = format!("{}/v1/models", provider.base_url.trim_end_matches('/'));

    // Anthropic uses a different authentication header scheme.
    let req = match provider.protocol.as_str() {
        "anthropic" => client
            .get(&models_url)
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2024-06-01"),
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
                ProbeResult::Error
            }
        }
        Err(_) => ProbeResult::Error,
    }
}

/// Outcome of a provider probe request.
enum ProbeResult {
    /// The provider responded successfully — it can be re-enabled.
    Success,
    /// The provider returned HTTP 429 — still rate-limited.
    RateLimited,
    /// The request failed or returned a non-success / non-429 status.
    Error,
}

/// Updates the router entry state based on the probe result.
///
/// - **Success**: re-enables the entry, resets cooldown to
///   [`BASE_COOLDOWN_SECONDS`].
/// - **RateLimited**: doubles the cooldown duration (capped at
///   [`MAX_COOLDOWN_SECONDS`]) and reschedules.
/// - **Error**: retries after the same cooldown duration without doubling.
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
            // Exponential backoff: double the cooldown, capped at the maximum.
            let new_duration = (current_cooldown * 2).min(MAX_COOLDOWN_SECONDS);
            entry_state.cooldown_until = Some(Utc::now() + chrono::Duration::seconds(new_duration));
            entry_state.cooldown_duration_seconds = Some(new_duration);
        }
        ProbeResult::Error => {
            // Non-429 errors keep the same cooldown duration and just retry.
            let new_until = Utc::now() + chrono::Duration::seconds(current_cooldown);
            entry_state.cooldown_until = Some(new_until);
            entry_state.cooldown_duration_seconds = Some(current_cooldown);
        }
    }
}

/// Manually enables or disables a specific entry within a group.
///
/// When enabling, the entry is set to [`EntryStatus::Active`] and all cooldown
/// state is cleared. When disabling, the entry is set to
/// [`EntryStatus::ManuallyDisabled`].
///
/// The `enabled` flag is also persisted to the group configuration so that the
/// change survives restarts.
///
/// # Arguments
///
/// - `state` — Shared router state containing all entry statuses.
/// - `groups` — The current group configuration (needed to locate the entry).
/// - `group_id` — The group containing the entry.
/// - `entry_index` — Zero-based index of the entry within the group.
/// - `enabled` — `true` to activate, `false` to disable.
///
/// # Errors
///
/// Returns an error string if the group or entry cannot be found, or if
/// persisting the updated configuration fails.
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

    // Persist the enabled flag so it survives a restart.
    let mut updated_groups = (*groups).clone();
    if let Some(g) = updated_groups.iter_mut().find(|g| g.id == group_id) {
        g.entries[entry_index].enabled = enabled;
    }
    store::save_groups(&updated_groups).map_err(|e| e.to_string())?;

    Ok(())
}