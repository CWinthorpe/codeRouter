# CodeRouter — Audit Report 001

**Date:** 2026-04-08  
**Scope:** Full project audit — bugs, design gaps, security, and completeness  
**Reference:** `docs/plan.md` (design spec), `docs/rules.md`, `docs/memories.md`  
**Build Status:** 76/76 Rust tests passing, TypeScript check clean

---

## Executive Summary

CodeRouter is a Tauri 2.x desktop application (React/TypeScript frontend + Rust Axum sidecar) that acts as a local OpenAI-compatible proxy router with multi-provider aggregation, model groups with failover, usage tracking, and OpenCode integration. All 18 build steps are complete and tests pass.

**This audit identifies 43 findings across 7 categories.** The most critical issues are:

- **Metrics recording is entirely non-functional** — the `MetricsRecorder` is defined but never instantiated or called in the request flow. No usage data is ever written to SQLite.
- **Token quota enforcement is dead code** — `record_success` is always called with `0` tokens, so `daily_tokens_used` never increments and quota checks never trigger.
- **Double scheduler spawn** — `spawn_scheduler` is called twice in `server.rs`, creating two concurrent scheduler loops that race on shared state and send duplicate probe requests.
- **OpenAI streaming SSE is corrupted** — raw upstream byte chunks are double-wrapped in SSE `data:` encoding, breaking client-side parsing.
- **shadcn/ui was never set up** — the plan specifies shadcn/ui + Tailwind, but only raw Tailwind utility classes are used.

---

## Findings

### Critical Bugs

| ID | Severity | File | Description |
|---|---|---|---|
| BUG-001 | Critical | `sidecar/src/proxy/server.rs:69,87` | **Double scheduler spawn** — `spawn_scheduler` is called twice, creating two independent scheduler loops that concurrently mutate `SharedRouterState` and send duplicate probe requests. Causes race conditions on quota reset and doubled timer intervals. |
| BUG-002 | Critical | `sidecar/src/proxy/server.rs:248` | **Metrics recording absent from request flow** — `MetricsRecorder` is defined in `metrics/recorder.rs` but never instantiated or used in `server.rs`. The plan states "On success: record usage metrics" — this step is entirely missing. No usage data is ever written to SQLite. |
| BUG-003 | Critical | `sidecar/src/proxy/server.rs:248` | **Token quota enforcement is dead code** — `record_success(&mut rs, &provider.id, entry_index, 0)` always passes `0` for `tokens_used`. `daily_tokens_used += 0` means the quota check (`daily_tokens_used >= quota`) will never trigger from actual usage. |
| BUG-004 | Critical | `sidecar/src/proxy/server.rs:369-377` | **OpenAI streaming SSE corruption** — raw upstream byte chunks (which already contain `data: {...}\n\n` lines) are wrapped in a single `Event::default().data(...)`, producing double-encoded SSE (`data: data: {...}\n\n`). Clients cannot parse the stream. |
| BUG-005 | Critical | `src/pages/Providers.tsx:157-165` | **API key overwritten on provider toggle** — `handleToggleEnabled` calls `saveProvider(updated, '')` with an empty string. The Rust backend calls `store_credential(&provider.id, "")`, overwriting the actual stored API key with an empty value. **Data loss bug.** |
| BUG-006 | Critical | `sidecar/src/proxy/router.rs:242-257` | **Consecutive error counter never resets after triggering** — after triggering cooldown, `consecutive_errors` stays at the threshold value. When cooldown expires and the entry becomes active again, the next single error immediately re-triggers cooldown (counter is already >= threshold). |

### High-Severity Bugs

| ID | Severity | File | Description |
|---|---|---|---|
| BUG-007 | High | `sidecar/src/metrics/scheduler.rs:48-54` | **ProbeGuard drop leaks keys on lock contention** — `try_lock()` can fail if another task holds the mutex. When it fails, the key is never removed from `ProbeLock::in_flight`, permanently blocking all future probes for that entry. |
| BUG-008 | High | `sidecar/src/metrics/scheduler.rs:116-119` | **Quota reset fires continuously after reset hour** — `should_reset` is `now >= t`, which is true for every tick after the reset hour passes until midnight. Entries get reset repeatedly within the same day. Should use `daily_reset_at` to gate. |
| BUG-009 | High | `sidecar/src/proxy/server.rs:394,418` | **`unwrap_or(Value::Null)` silently swallows JSON parse errors** — if upstream returns invalid JSON, processing continues with `Value::Null`, producing a malformed response to the client. |
| BUG-010 | High | `sidecar/src/proxy/server.rs:248` | **No latency tracking** — `record_success` is called with `0` tokens and no timing is measured anywhere in `route_request`. `RequestEvent.latency_ms` can never be populated. |
| BUG-011 | High | `src/pages/Dashboard.tsx:369` | **Missing React Fragment in RequestFeed** — `requests.map()` returns two adjacent `<tr>` elements per iteration without a wrapping `<>...</>`, causing "unique key" warnings and potentially broken rendering. |
| BUG-012 | High | `src/pages/ModelGroups.tsx:276` | **`activeEntry` logic is flawed** — assumes `entry_index === 0` means highest-priority active entry, but `entry_index` is the index within the group's entry list, not the routing target. If priority 0 is in cooldown but priority 1 is active, the wrong provider is shown. |
| BUG-013 | High | `src/pages/UsageMetrics.tsx:271-283` | **`SortHeader` and `FilterDropdown` defined inside render** — causes recreation on every render, breaking React reconciliation and causing unnecessary unmounts/remounts. |

### Medium-Severity Bugs

| ID | Severity | File | Description |
|---|---|---|---|
| BUG-014 | Medium | `sidecar/src/config/store.rs:32` | **Atomic write uses fixed `.tmp` extension** — `path.with_extension("tmp")` means concurrent writes to different files in the same directory use the same temp filename if basenames differ only by extension. Collision risk. |
| BUG-015 | Medium | `sidecar/src/proxy/server.rs:254` | **429 backoff never doubles on successive hits** — `record_429(..., 60)` always passes 60. Exponential backoff doubling only happens during recovery probes, not on successive 429s during active routing. Plan: "doubling up to 1 hour max". |
| BUG-016 | Medium | `src/pages/ModelGroups.tsx:142` | **Dead code `openCodeRef`** — `const openCodeRef = false;` means the OpenCode reference warning in delete confirmation is never shown. Should check if the group is actually referenced in OpenCode config. |
| BUG-017 | Medium | `src/pages/UsageMetrics.tsx:146` | **Unsafe `sortColumn` cast** — `sortColumn as keyof RequestRow` where `sortColumn` is typed as `string`. If a column name doesn't exist on `RequestRow`, this produces `undefined` values and incorrect sorting. |
| BUG-018 | Medium | `src/pages/OpenCodeSetup.tsx:96-104` | **Stale closure in debounced preview** — `useEffect` references `fetchPreview` but it's not in the dependency array. Can cause stale closures. |
| BUG-019 | Medium | `src/pages/Dashboard.tsx:12,9` | **Hardcoded health URL** — `PROXY_HEALTH_URL` is hardcoded to `localhost:4141/health`. If user changes proxy port in settings, health checks will hit the wrong port. Same issue in `useProxyStatusPoll.ts`. |
| BUG-020 | Medium | `src-tauri/src/main.rs:97` | **`poll_health` uses hardcoded port 4141** — if user changes proxy port in settings, health check still hits 4141 and incorrectly reports proxy as stopped. |

### Design Gaps (Plan vs Implementation)

| ID | Severity | Area | Description |
|---|---|---|---|
| GAP-001 | High | Backend | **`proxy::upstream` module missing** — plan lists it for "HTTP client pool, upstream request dispatch, timeout handling". Logic is inlined in `server.rs`. |
| GAP-002 | High | Backend | **Model refresher never scheduled** — `refresh_all_providers` exists but is never called from server or scheduler. Plan: "On provider add (and on a daily refresh schedule)". |
| GAP-003 | High | Backend | **`/v1/models` missing metadata** — plan: "Each alias appears as a standard OpenAI model object with metadata derived from the highest-priority active provider." Implementation returns only `id`, `object`, `created: 0`, `owned_by`. No `context_window` or `max_output_tokens`. |
| GAP-004 | High | Backend | **System agents missing from `AgentMapping`** — plan lists 7 agents: `build`, `plan`, `general`, `explore`, `compaction`, `title`, `summary`. `AgentMapping` only has `build`, `plan`, `general`, `explore`, `small_model`. |
| GAP-005 | Medium | Backend | **`opencode.json` cached config not implemented** — plan lists `~/.config/coderouter/opencode.json` as "Cached OpenCode integration settings". No such file is read or written. |
| GAP-006 | Medium | Backend | **No latency percentile tracking (p50, p95)** — plan: "Latency percentiles (p50, p95) per day". DB stores per-request `latency_ms` but no percentile computation exists. |
| GAP-007 | Medium | Backend | **Daily rolling totals not persisted** — plan: "Daily rolling totals (reset at provider's configured quota reset time)". `daily_tokens_used` is in-memory only (`EntryState`), lost on restart. |
| GAP-008 | Medium | Backend | **`/v1/completions` is bare passthrough** — just calls `route_request` with `"completions"`. No Anthropic translation, no model routing logic for legacy completions format. |
| GAP-009 | Medium | Backend | **`/health` endpoint incomplete** — plan: "Returns proxy status, active providers, current failover states". Implementation only returns `status`, `proxy`, `uptime_seconds`. |
| GAP-010 | High | Frontend | **shadcn/ui not configured** — plan specifies "shadcn/ui + Tailwind CSS". No `components/ui/` directory, no `components.json`, no shadcn CSS variables. All components use raw Tailwind utility classes. |
| GAP-011 | Medium | Frontend | **OpenCode config path is display-only, not functional** — plan: "Detected OpenCode config path (editable)". Input is editable but `manualPath` state is never sent to backend. All IPC commands use `detect_opencode_config()` internally, ignoring manual path. |
| GAP-012 | Low | Frontend | **Missing dedicated "Remove CodeRouter from OpenCode config" button** — plan explicitly requires this button. Toggle serves similar purpose but plan calls for separate explicit button. |

### Security Issues

| ID | Severity | File | Description |
|---|---|---|---|
| SEC-001 | High | `sidecar/src/proxy/server.rs:206-212` | **No input validation on `base_url` — SSRF risk** — `provider.base_url` is used directly in URL construction with no validation. A malicious config with `base_url: "http://169.254.169.254"` would be forwarded to. |
| SEC-002 | Medium | `sidecar/src/config/store.rs:40-52` | **Config files written without restrictive permissions** — files created with default umask (typically 0644). Config files containing credential key references should be 0600. |
| SEC-003 | Medium | `sidecar/src/opencode/config_writer.rs:335-344` | **OpenCode config writer has no file locking or atomic write** — uses plain `fs::write`, not atomic write with file locks like `config::store` does. Concurrent writes could corrupt the file; crash mid-write leaves corrupted file. |
| SEC-004 | Low | `sidecar/src/proxy/translator.rs:451` | **Anthropic API version hardcoded to `2023-06-01`** — oldest version. Works but misses newer features. |

### Schema Mismatches (plan.md JSON vs Code)

| ID | Severity | File | Description |
|---|---|---|---|
| SCH-001 | Medium | `sidecar/src/config/models.rs:69-82` | **`FailoverConfig` fields use snake_case, plan specifies camelCase** — no `#[serde(rename)]` on any field. Plan: `on429`, `onQuotaExhausted`, `onConsecutiveErrors`, `consecutiveErrorThreshold`, `onLatencyTimeout`, `latencyTimeoutMs`. Code serializes as: `on_429`, `on_quota_exhausted`, `on_consecutive_errors`, `consecutive_error_threshold`, `on_latency_timeout`, `latency_timeout_ms`. |
| SCH-002 | Medium | `sidecar/src/config/models.rs:27-30` | **`Provider.daily_token_quota` and `quota_reset_utc_hour` — no rename** — serialize as snake_case, plan specifies `dailyTokenQuota` and `quotaResetUtcHour`. |
| SCH-003 | Medium | `sidecar/src/config/models.rs:54-61` | **`GroupEntry.daily_token_quota_override` and `cooldown_until` — no rename** — serialize as snake_case, plan specifies `dailyTokenQuotaOverride` and `cooldownUntil`. |

### Failover & Recovery Issues

| ID | Severity | File | Description |
|---|---|---|---|
| FAI-001 | Medium | `sidecar/src/metrics/scheduler.rs:161-163` | **`QuotaExhausted` entries are never probed for recovery** — `run_cooldown_check` only probes `EntryStatus::Cooldown` entries. `QuotaExhausted` entries rely solely on quota reset timer. If timer fails or quota is manually increased, entry stays exhausted forever. |
| FAI-002 | Medium | `sidecar/src/metrics/scheduler.rs:312-318` | **`set_entry_enabled` only persists disable, not enable** — `save_groups` is only called when `enabled == false`. Enabling in UI updates runtime state but doesn't persist `enabled: true` to `groups.json`. |
| FAI-003 | Low | `sidecar/src/proxy/router.rs:259-265` | **`record_latency_timeout` ignores config internally** — function always sets cooldown. Caller guards the call, but function itself has no awareness of config. If called from elsewhere, would always trigger. |

### Tauri / Integration Issues

| ID | Severity | File | Description |
|---|---|---|---|
| TAU-001 | Medium | `src-tauri/src/commands.rs:434-447` | **`restart_proxy` doesn't update tray/menu state** — sets `proxy_running = true` but doesn't call `update_tray_icon` or `update_menu_labels`. Tray icon and menu labels won't reflect new state until next `poll_health` tick (up to 5 seconds). |
| TAU-002 | Medium | `src-tauri/src/commands.rs:421-426` | **Sidecar spawn path assumes release layout** — `current_exe().parent().join("sidecar/coderouter-proxy")` — in an AppImage, the sidecar binary layout may differ. |
| TAU-003 | Low | `src/lib/ipc.ts` | **`get_usage_by_day` and `get_usage_by_group` defined in Rust but not in `ipc.ts`** — dead backend endpoints with no TypeScript bindings or frontend consumers. |

### Frontend Code Quality Issues

| ID | Severity | File | Description |
|---|---|---|---|
| FE-001 | Medium | `src/store/index.ts:36-38` | **`loadInitialData` silently swallows all errors** — empty `catch {}` means if IPC fails in production, UI shows empty state with no error indication. Should at least log or set an error flag. |
| FE-002 | Low | `src/types/index.ts` | **Loose string types** — `GroupEntry.status`, `EntryStatusResponse.status`, `Provider.protocol`, `AppConfig.log_verbosity`, `RequestRow.status` are all typed as `string` instead of discriminated unions. Should be `'active' \| 'cooldown' \| ...` etc. |
| FE-003 | Low | Multiple pages | **Duplicated components** — `Toast` defined inline in `Providers.tsx:645`, `ModelGroups.tsx:962`, `OpenCodeSetup.tsx:373`. `ActionButton` duplicated in `Providers.tsx:345` and `ModelGroups.tsx:351`. Should be extracted to shared components. |
| FE-004 | Low | `src/pages/OpenCodeSetup.tsx:203-217` | **Config path editable but not saved** — user can type a custom path but it's never used. All backend calls use auto-detection. |

---

## Summary by Severity

| Severity | Count |
|---|---|
| Critical | 6 |
| High | 10 |
| Medium | 18 |
| Low | 9 |
| **Total** | **43** |

## Summary by Category

| Category | Count |
|---|---|
| Bugs | 20 |
| Design Gaps | 12 |
| Security | 4 |
| Schema Mismatches | 3 |
| Failover/Recovery | 3 |
| Tauri/Integration | 3 |
| Frontend Quality | 4 |

---

## Recommended Priority Order for Fixes

1. **BUG-002** — Wire up `MetricsRecorder` in the request flow (core functionality broken)
2. **BUG-003** — Pass actual token counts to `record_success` (quota enforcement broken)
3. **BUG-001** — Remove duplicate `spawn_scheduler` call (race condition)
4. **BUG-004** — Fix OpenAI streaming SSE passthrough (streaming broken for non-Anthropic providers)
5. **BUG-005** — Fix `handleToggleEnabled` to not pass empty API key (data loss)
6. **BUG-006** — Reset `consecutive_errors` counter after triggering cooldown (failover broken)
7. **GAP-010** — Set up shadcn/ui or update plan to reflect Tailwind-only approach
8. **GAP-002** — Schedule model refresher (daily refresh never runs)
9. **SEC-001** — Validate `base_url` to prevent SSRF
10. **SCH-001–003** — Add `#[serde(rename)]` to match plan's JSON schema

---

## Notes

- All 76 Rust unit tests pass. TypeScript type check is clean.
- No implementation notes exist in `implementation-notes/` (directory empty).
- No AI prompt files exist in `ai-prompts/` (directory empty).
- This is Audit 001 — subsequent audits should track remediation of these findings.
