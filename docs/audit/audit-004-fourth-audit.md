# CodeRouter — Audit Report 004

**Date:** 2026-04-08  
**Scope:** Full project re-audit — bugs, design gaps, security, and completeness  
**Reference:** `docs/plan.md` (design spec), `docs/audit/audit-003-third-audit.md` (prior audit)  
**Build Status:** 103 Rust tests passing (12 types + 91 sidecar), TypeScript check clean

---

## Executive Summary

This is the fourth audit of CodeRouter. Test count has grown from 76 → 86 → 102 → 103. Many issues from prior audits have been addressed. **52 findings** remain across the Rust sidecar, React frontend, and project infrastructure.

**The most impactful remaining issues are:**

- **Streaming body has no timeout** — Once the response stream starts, a stalled connection hangs forever. The plan specifies latency timeout as a failover trigger, but it cannot fire for streaming bodies.
- **Config changes from Tauri UI are not visible to the sidecar proxy** — `save_provider`, `save_group`, `delete_provider`, etc. write to disk, but the sidecar's `AppState` caches groups/providers in `Arc<RwLock<...>>` that are only reloaded in `handle_internal_router_set_entry`. New providers, updated groups, and changed settings are invisible to the proxy until restart.
- **`daily_requests_used` not initialized from DB on startup** — `init_daily_totals_from_db` only initializes `daily_tokens_used`. Request counts stay at 0 after restart, so daily request quotas are not enforced.
- **`kill_sidecar` sends SIGKILL** — Bypasses the sidecar's graceful shutdown handler. Metrics are lost, background tasks abruptly terminated.
- **Failed streaming responses are not recorded as errors** — If the inner stream returns an error, `on_complete` is never called. Metrics are not recorded, `consecutive_errors` is not incremented, and the router state is not updated. A provider that consistently returns streaming errors would never trigger failover.
- **`ProviderModal` loses `dailyRequestQuota` and `modelOverrides` on edit** — These fields are not included in the `providerObj` constructed during save, causing data loss.

---

## Previous Audit Fix Verification

| Audit 003 Issue | Status | Notes |
|---|---|---|
| FIND-001/002: Streaming token extraction | ✅ Fixed | Anthropic: token counts extracted via closure capture. OpenAI: tokens parsed from SSE stream. |
| FIND-003: QuotaExhausted status never set | ✅ Fixed | `process_response` now checks token quota and returns `QuotaExhausted` |
| FIND-004: In-memory groups stale after set_entry_enabled | ⚠️ Partially — groups/providers reloaded but router state `entries` map not updated for new entries (see FIND-010, FIND-011) |
| FIND-005: consecutive_errors not reset on quota recovery | ✅ Fixed | Scheduler now resets consecutive_errors |
| FIND-006: consecutive_errors not reset on latency timeout | ✅ Fixed | `record_latency_timeout` now resets consecutive_errors |
| FIND-007: Scheduler probe has no timeout | ✅ Fixed | Probe now has 10s timeout |
| FIND-008: AppImage sidecar path detection | ✅ Fixed | Now uses `APPDIR` env var |
| FIND-009: FilterDropdown click-outside re-subscribes | ✅ Fixed | `onToggle` now memoized |
| FIND-010: record_success doesn't check quota after increment | ⚠️ Partially — quota check added but concurrent requests can still slip through (see FIND-013) |
| FIND-011: handle_models doesn't check quota | ✅ Fixed | Now checks quota before including group |
| FIND-012: handle_models uses stale in-memory groups | ❌ **NOT FIXED** — Still uses `state.groups` loaded at startup; config changes from Tauri UI not visible (see FIND-002) |
| FIND-013: run_cooldown_check holds lock during iteration | ⚠️ Partially — scope reduced but lock still held (see FIND-014) |
| FIND-014: process_response reads error body for streaming | ⚠️ Partially — status check added but body handling incomplete (see FIND-015) |
| FIND-015: route_request reloads providers every request | ⚠️ Partially — still reloads from disk, inconsistent with in-memory groups (see FIND-002) |
| FIND-016: record_consecutive_error only tracks when enabled | ⚠️ Partially — now tracks always but counter behavior on re-enable needs review |
| FIND-017: Model refresher no timeout on detail requests | ✅ Fixed | Per-request timeout added |
| FIND-018: shadcn/ui barely used | ❌ **NOT FIXED** — Still barely used (see FIND-035) |
| FIND-019: Provider type missing model_overrides | ✅ Fixed | TypeScript type now includes model_overrides |
| FIND-020: AppConfig type missing opencode_config_path | ✅ Fixed | TypeScript type now includes opencode_config_path |
| FIND-021: Tauri command test coverage thin | ⚠️ Partially — tests added but edge cases not covered |
| FIND-022: Zero frontend tests | ⚠️ Partially — type tests and store tests added, but no component tests |
| FIND-023: OpenOptions without truncate | ❌ **NOT FIXED** — Still uses `.create(true)` without `.truncate(true)` (see FIND-040) |
| FIND-024: save_provider unwrap_or_default | ✅ Fixed | Now returns error instead of silently starting fresh |
| FIND-025: Background tasks no graceful shutdown | ❌ **NOT FIXED** — SIGKILL from Tauri bypasses handler (see FIND-004) |
| FIND-026: Build script issues | ⚠️ Partially — some fixed but `npm install` still missing (see FIND-043) |
| FIND-027: No workspace dependencies | ⚠️ Partially — some added but not all shared deps unified |
| FIND-028: ProviderHealthCards summaries never refresh | ✅ Fixed | Now refreshes on interval |
| FIND-029: Duplicate group status polling | ✅ Fixed | Single shared poll per group |
| FIND-030: handleDragOver excessive re-renders | ✅ Fixed | Throttle added |
| FIND-031: loadInitialData fails atomically | ✅ Fixed | Now uses `Promise.allSettled` |
| FIND-032: Unnecessary dynamic imports | ✅ Fixed | Now uses static imports |
| FIND-033: Drag-and-drop key collision | ⚠️ Partially — key improved but can still collide for same provider+model (see FIND-027) |

---

## Critical Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-001 | Critical | `sidecar/src/proxy/server.rs:554` | **Streaming body has no timeout** — `resp.bytes_stream()` is used without any timeout wrapper. `send_with_timeout` only covers getting response headers. Once the stream starts, a stalled connection hangs forever. The plan specifies latency timeout as a failover trigger, but it cannot fire for streaming bodies. |
| FIND-002 | Critical | `src-tauri/src/commands.rs:96-188`, `sidecar/src/proxy/server.rs:79-86` | **Config changes from Tauri UI are not visible to the sidecar proxy** — Tauri commands (`save_provider`, `save_group`, `delete_provider`, `delete_group`, `toggle_provider_enabled`) write config to disk, but the sidecar's `AppState` caches `groups` and `providers` in `Arc<RwLock<Arc<...>>>`. These are only reloaded in `handle_internal_router_set_entry`. New providers, updated groups, and changed settings are invisible to the proxy until it is restarted. |
| FIND-003 | Critical | `sidecar/src/proxy/router.rs:101-131` | **`daily_requests_used` not initialized from DB on startup** — `init_daily_totals_from_db` only initializes `daily_tokens_used`. `daily_requests_used` stays at 0 after restart, so daily request quotas are not enforced until the next scheduler reset cycle. |
| FIND-004 | Critical | `src-tauri/src/commands.rs:516` | **`kill_sidecar` sends SIGKILL — no graceful shutdown** — `Child::kill()` sends SIGKILL on Linux. The sidecar's graceful shutdown handler (SIGTERM/SIGINT in `server.rs`) is never triggered. Metrics are lost, background tasks are abruptly terminated. |
| FIND-005 | Critical | `sidecar/src/proxy/server.rs:667-684` | **Failed streaming responses are not recorded as errors** — If the inner stream returns `Poll::Ready(Some(Err(e)))`, the error is passed through but `on_complete` is never called. Metrics are not recorded, `consecutive_errors` is not incremented, and the router state is not updated. A provider that consistently returns streaming errors would never trigger failover. |

## High-Severity Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-006 | High | `sidecar/src/proxy/server.rs:676-679` | **`MetricsRecordingStream` callback always called with `(0, 0)`** — The callback at line 380 ignores these arguments and reads from `token_counts` via closure capture. This works by accident but the function signature is misleading. If the callback were changed to use the arguments, it would silently record 0 tokens. |
| FIND-007 | High | `sidecar/src/models/refresher.rs:99` | **Model detail requests have no authentication header** — The `/v1/models/{model_id}` detail request has no `Authorization` header. Most OpenAI-compatible providers require auth for this endpoint, so model metadata will never be fetched. |
| FIND-008 | High | `sidecar/src/models/refresher.rs:126-128` | **`extract_from_raw_json` is dead code** — `parse_model_detail` always returns `Ok(...)` for any valid JSON (uses `unwrap_or` to create a default struct). The `else if` branch calling `extract_from_raw_json` is only reached for invalid JSON, where `serde_json::from_str` would also fail. The entire function is unreachable. |
| FIND-009 | High | `sidecar/src/proxy/router.rs:183-185` | **`select_entry` doesn't create `EntryState` for new entries added at runtime** — If a new entry is added to a group via the UI, there's no corresponding `EntryState` in the router's `entries` map. `select_entry` falls through to `true`, treating it as eligible — but `record_success`, `record_429`, etc. silently fail to find the entry. |
| FIND-010 | High | `sidecar/src/proxy/server.rs:797-825` | **`handle_internal_router_set_entry` doesn't create `EntryState` for new entries** — When an entry is enabled, groups/providers are reloaded from disk, but the router state's `entries` map is not updated. If the entry didn't exist before, it still won't exist in the map. |
| FIND-011 | High | `sidecar/src/metrics/scheduler.rs:313-317` | **`set_entry_enabled` fails for entries not in router state** — If the entry was added after the sidecar started, `get_mut(&key)` returns an error. |
| FIND-012 | High | `src/pages/Providers.tsx:466-478` | **`ProviderModal` loses `dailyRequestQuota` on edit** — The field is not included in the `providerObj` constructed during save. When editing a provider with a daily request quota set, it will be lost. |
| FIND-013 | High | `src/pages/Providers.tsx:466-478` | **`ProviderModal` loses `modelOverrides` on edit** — The field is not included in the `providerObj` constructed during save. When editing a provider with model overrides set, they will be lost. |

## Medium-Severity Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-014 | Medium | `sidecar/src/metrics/scheduler.rs:146-187` | **`run_cooldown_check` holds router state lock while iterating all groups** — The lock is held for the entire iteration. During this time, the proxy server cannot access router state for request routing. With many groups/entries, this causes latency spikes every 30 seconds. |
| FIND-015 | Medium | `sidecar/src/proxy/server.rs:422-425` | **`process_response` reads error body for streaming responses** — For streaming responses that return a non-success status, `resp.text().await` is called. If the upstream returned a streaming error response, this could hang indefinitely. |
| FIND-016 | Medium | `sidecar/src/proxy/router.rs:268-271` | **`record_success` does not reset `cooldown_duration_seconds` on Cooldown→Active transition** — When a probe request succeeds and transitions from `Cooldown` to `Active`, the backoff duration is not reset. The scheduler's `handle_probe_result` does reset it, but `record_success` does not. If a regular request succeeds during cooldown, the next 429 would start from the doubled value instead of 60s. |
| FIND-017 | Medium | `sidecar/src/proxy/router.rs:304-309` | **`record_quota_exhausted` does not reset `consecutive_errors`** — If a provider accumulates 4 consecutive errors then hits quota exhaustion, the error count persists. The scheduler clears it at reset time, but between exhaustion and reset the count is stale. |
| FIND-018 | Medium | `sidecar/src/proxy/translator.rs:400, 546` | **SSE parsing only splits on `\n`, not `\r\n`** — If an upstream provider sends `\r\n` line endings, the `\r` is included in the data string, potentially causing JSON parse failures or corrupted output. |
| FIND-019 | Medium | `sidecar/src/proxy/router.rs:115-130` | **`init_daily_totals_from_db` double-counts for providers with entries in multiple groups** — For each provider, `get_today_token_totals` returns the total tokens across ALL groups. Then for EACH entry matching that provider_id, it sets `daily_tokens_used = tokens`. If provider "p1" has entries in 2 groups and used 1000 tokens total, both entries get `daily_tokens_used = 1000`. Correct for quota enforcement but semantically confusing. |
| FIND-020 | Medium | `sidecar/src/proxy/server.rs:73, 75` | **`start_server` calls `init_db()` twice** — `init_daily_totals_from_db` calls `init_db()` internally, then `metrics_db::init_db()` is called again. Two separate SQLite connections to the same file. |
| FIND-021 | Medium | `sidecar/src/proxy/server.rs:755-757` | **`AppError::InternalError` leaks internal details to client** — The error message is included verbatim in the JSON response body. Internal error details (file paths, DB errors, etc.) are exposed to the client. |
| FIND-022 | Medium | `sidecar/src/proxy/server.rs:246` | **`handle_models` endpoint uses current timestamp for `created` field** — Returns `chrono::Utc::now().timestamp()` on every call instead of a fixed creation timestamp. Deviates from OpenAI API spec. |
| FIND-023 | Medium | `src-tauri/src/commands.rs:33-49` | **`ProviderResponse` struct missing `daily_request_quota` field** — The `Provider` struct has `daily_request_quota`, but `ProviderResponse` does not include it. The UI cannot display request quotas. |
| FIND-024 | Medium | `sidecar/src/config/store.rs:32-68` | **`atomic_write` leaves temp files on failure** — If the write fails after creating the temp file but before `fs::rename`, the `.tmp.{pid}` file is left behind. |
| FIND-025 | Medium | `sidecar/src/metrics/queries.rs:48-57, 121-128, 194-206` | **Daily summary/usage/latency queries use midnight UTC, not provider reset hour** — Only `get_today_token_totals` uses the provider's `quota_reset_utc_hour`. All other daily queries use midnight UTC, meaning the UI's daily totals may not match the provider's actual billing cycle. |
| FIND-026 | Medium | `sidecar/src/models/refresher.rs:88-131` | **`fetch_openai_compatible_models` fetches model details sequentially** — Each model detail request is made one at a time. For providers with 100+ models, this could take minutes. |
| FIND-027 | Medium | `src/pages/ModelGroups.tsx:714` | **`GroupForm` drag-and-drop key can still collide** — Key is `${entry.providerId}-${entry.modelId}-${idx}-${entry.priority}`. If the same provider+model is added twice (valid use case — same model at different priorities), the key collides because `idx` and `priority` are the same for both entries. |
| FIND-028 | Medium | `src/pages/UsageMetrics.tsx:174-175, 202` | **Timezone mismatch in custom date range inputs** — `formatDate` uses `toISOString().slice(0, 10)` (UTC) but `<input type="date">` uses local time. For users in non-UTC timezones, the displayed date and actual value mismatch. |
| FIND-029 | Medium | `src/pages/Settings.tsx:114-130` | **`handleResetSettings` doesn't clear providers/groups in Zustand store** — After `resetAllConfig()`, the providers and groups in the store are not cleared. Old data shows until next `loadInitialData()` or page refresh. |
| FIND-030 | Medium | `src-tauri/tauri.conf.json:22` | **CSP allows `http://localhost:*` — wildcard port** — The `connect-src` allows connections to any port on localhost, which defeats the purpose of SSRF protection. Should be restricted to known ports (e.g., `http://localhost:4141`). |
| FIND-031 | Medium | `src-tauri/tauri.conf.json:12` | **`withGlobalTauri` is `true`** — Exposes `window.__TAURI__` globally. Deprecated in Tauri v2 in favor of explicit imports. Larger attack surface. |
| FIND-032 | Medium | `build.sh:39` | **Hardcoded x86_64 pkg-config path** — `PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig` is hardcoded for Debian/Ubuntu x86_64. Will fail on ARM64, Fedora, Arch, or other distros. |
| FIND-033 | Medium | `build.sh` | **No `npm install` / `npm ci` step** — The script assumes `node_modules` already exist. A clean checkout would fail at `npm run tauri build`. |
| FIND-034 | Medium | `sidecar/src/config/store.rs:123` | **`OpenOptions` with `.create(true)` but no `.truncate(true)`** — If the file already exists with content, subsequent `read_to_string` + `serde_json::from_str` could fail on partial overwrites. `set_len(0)` is called later but there's a read between open and truncate that could be stale. |
| FIND-035 | Medium | `src/components/ui/` (8 components) | **shadcn/ui components installed but barely used** — `Dialog`, `Select`, `Table`, `Tabs`, `Progress` are never used. All pages use custom implementations. |
| FIND-036 | Medium | `src/pages/Dashboard.tsx:84-90` | **`quota_exhausted` status not counted in Dashboard provider cards** — `getEntryStatusCounts` only counts `active`, `cooldown`, and `manually_disabled`. `quota_exhausted` entries are silently ignored. |
| FIND-037 | Medium | `src-tauri/src/commands.rs`, `src/` | **Frontend component test coverage is minimal** — Type tests verify object literals (which TypeScript already guarantees). Store tests mock IPC but don't test real behavior. No component/page tests exist. |
| FIND-038 | Medium | `src/lib/ipc.ts` | **`get_usage_by_day`, `get_usage_by_group`, `get_latency_percentiles` IPC commands not wired to frontend** — Defined in Rust but no TypeScript bindings or frontend consumers. UsageMetrics aggregates client-side from raw data, which is inefficient for large datasets. |
| FIND-039 | Medium | `src/pages/Providers.tsx:160-168` | **`handleToggleEnabled` doesn't show toast feedback** — Unlike save, delete, test connection, and refresh models, the toggle handler provides no user feedback. |
| FIND-040 | Medium | `src/pages/ModelGroups.tsx:427` | **`LiveStatusPanel` status fallback doesn't account for `quota_exhausted`** — If status data is unavailable for an entry, it falls back to `'active'` or `'manually_disabled'`, not considering `quota_exhausted`. |
| FIND-041 | Medium | `src/pages/Dashboard.tsx:140-147` | **`ProxyStatusCard` shows "Stopped" for `'unknown'` status** — When `proxyStatus` is `'unknown'` (initial state before first poll), it shows as "Stopped" with red styling. Misleading — the app hasn't determined the status yet. |
| FIND-042 | Medium | `src-tauri/src/main.rs:97-124` | **`poll_health` runs forever with no cancellation** — Infinite loop with no cancellation mechanism. When the app exits, this task is not gracefully cancelled. |
| FIND-043 | Medium | `sidecar/src/proxy/server.rs:76, 143` | **MetricsRecorder handle not awaited on shutdown** — `_metrics_handle` is dropped immediately. On shutdown, the recorder channel is closed but the handle is never `.await`ed — no guarantee pending metrics are flushed before process exits. |

## Low-Severity Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-044 | Low | `sidecar/src/metrics/scheduler.rs:131-136` | `run_quota_reset` has unused `_today_reset` variable |
| FIND-045 | Low | `sidecar/src/metrics/scheduler.rs:64` | `spawn_scheduler` has unused `_client_clone` variable |
| FIND-046 | Low | `sidecar/src/proxy/server.rs:528-530` | `process_response` for 429 doesn't consume response body |
| FIND-047 | Low | `sidecar/src/proxy/server.rs:298` | `route_request` clones entire providers vector on every request |
| FIND-048 | Low | `sidecar/src/models/refresher.rs:287` | `needs_refresh` only checks the first model's timestamp |
| FIND-049 | Low | `sidecar/src/models/refresher.rs:139-151` | `parse_model_detail` silently swallows deserialization errors |
| FIND-050 | Low | `sidecar/src/proxy/ssrf.rs:6-10` | SSRF validation only at config time, not at request time (DNS rebinding possible) |
| FIND-051 | Low | `sidecar/src/config/models.rs:142-144` | No input validation on proxy port (could be 0 or privileged) |
| FIND-052 | Low | `sidecar/src/proxy/server.rs:260-272` | `handle_chat_completions`/`handle_completions` don't validate request body structure |
| FIND-053 | Low | `sidecar/src/proxy/router.rs:327` | `record_consecutive_error` resets counter after triggering — exact count at threshold is lost |
| FIND-054 | Low | `sidecar/src/proxy/upstream.rs:32-72` | `build_completion_request` for Anthropic is best-effort translation (prompt → single user message) |
| FIND-055 | Low | `src-tauri/src/commands.rs:270-294` | `refresh_provider_models` doesn't update sidecar in-memory state |
| FIND-056 | Low | `src-tauri/src/commands.rs:138-157` | `delete_provider` doesn't clean up router state entries (orphaned `EntryState`) |
| FIND-057 | Low | `sidecar/src/opencode/config_writer.rs:108-137, 291-320` | `inject_provider` and `preview_opencode_config` duplicate model-building logic |
| FIND-058 | Low | `src-tauri/src/commands.rs:559` | `find_sidecar_fallback` looks for wrong binary name in debug mode |
| FIND-059 | Low | `sidecar/src/opencode/config_writer.rs:428-429` | `write_config` unlocks before setting permissions and renaming — crash window |
| FIND-060 | Low | `sidecar/src/proxy/translator.rs:605-608` | `estimate_tokens_from_text` uses crude char/4 heuristic |
| FIND-061 | Low | `sidecar/src/proxy/server.rs:695-696` | `handle_health` clones providers and groups on every call |
| FIND-062 | Low | `src/pages/Dashboard.tsx:65` | `formatRelativeTime` returns "just now" for future timestamps |
| FIND-063 | Low | `src/pages/ModelGroups.tsx:97` | `cooldownCountdown` has unused `_tick` parameter |
| FIND-064 | Low | `src/pages/Dashboard.tsx:121`, `src/pages/UsageMetrics.tsx:648` | `StatusBadge` import at bottom of file (style violation) |
| FIND-065 | Low | `src/pages/UsageMetrics.tsx:285-330` | `chartData` useMemo depends on `allRequests` but only uses it for color generation — could show empty series |
| FIND-066 | Low | `src/pages/OpenCodeSetup.tsx:131-142` | Provider status check on mount uses `previewOpencodeConfig` which generates a preview, not actual file state |
| FIND-067 | Low | `src/pages/Settings.tsx:54-56` | `updateField` not wrapped in `useCallback` |
| FIND-068 | Low | `src/pages/Providers.tsx:544-559` | API key Clear/Show buttons can overlap on narrow inputs |
| FIND-069 | Low | `src/types/index.ts:85` | `cooldown_duration_seconds` is `number \| undefined` but Rust sends `number \| null` |
| FIND-070 | Low | `src/components/AppShell.tsx:78` | `SidebarItem` icon uses `React.FC<...>` pattern (less preferred) |
| FIND-071 | Low | `src/pages/Onboarding.tsx:15` | Onboarding steps don't adapt to current state — user sees "Add Provider" step even when providers exist |
| FIND-072 | Low | `src/pages/Providers.tsx:488`, `ModelGroups.tsx:652`, `Onboarding.tsx:44` | Modals don't handle Escape key to close |
| FIND-073 | Low | All modal components | Modals missing `role="dialog"`, `aria-modal`, `aria-label` attributes |
| FIND-074 | Low | `package.json:42-43` | Both `happy-dom` and `jsdom` installed — `happy-dom` is unused dead weight |
| FIND-075 | Low | `Cargo.toml` (root) | Not all shared deps unified in `[workspace.dependencies]` |
| FIND-076 | Low | `Makefile:12-15` | `clean` target doesn't clean Tauri build artifacts (`src-tauri/gen/`, `src-tauri/sidecar/`, bundle outputs) |
| FIND-077 | Low | `src-tauri/src/commands.rs:737-740` | Test name `test_get_providers_returns_empty_when_no_config` is misleading — asserts `result.is_err()` |
| FIND-078 | Low | `src-tauri/src/main.rs:136` | `let active_icon = make_icon(false)` — variable name misleading (false = inactive icon) |
| FIND-079 | Low | `src/test/setup.ts:1` | Vitest setup file is minimal — no Tauri API mocks set up globally |
| FIND-080 | Low | `tsconfig.json` | Missing `tsconfig.node.json` for Vite config type-checking |

---

## Design Gaps vs plan.md

| ID | Area | Description |
|---|---|---|
| DG-001 | Backend | **`daily_request_quota` not enforced** — Plan mentions "daily request quota" as optional provider field. Field exists in struct but not enforced in routing. |
| DG-002 | Backend | **Latency timeout cooldown not configurable** — Plan: "Cooldown period (configurable, default 5 minutes)." Hardcoded to 5 minutes. |
| DG-003 | Backend | **Consecutive errors cooldown not configurable** — Plan: "Fixed cooldown period (configurable, default 10 minutes)." Hardcoded to 10 minutes. |
| DG-004 | Backend | **`/v1/completions` not "forwarded as-is"** — Plan: "Legacy completions endpoint (forwarded as-is if provider supports it)." Implementation translates to Anthropic messages format, losing system prompts and conversation history. |
| DG-005 | Backend | **OpenCode config writer uses static config status, not runtime state** — `inject_provider` and `preview_opencode_config` filter entries by `e.status == "active"` (static config string), not runtime `EntryStatus`. |
| DG-006 | Backend | **`opencode.json` cached config not implemented** — Plan lists `~/.config/coderouter/opencode.json`. Not read or written. |
| DG-007 | Backend | **System agents `compaction`, `title`, `summary` missing** — Plan lists 7 agents. `AgentMapping` only has `build`, `plan`, `general`, `explore`, `small_model`. |
| DG-008 | Frontend | **No first-run onboarding flow persistence** — Onboarding component exists but dismissal state not properly persisted — shows again on every launch until providers and groups are added. |
| DG-009 | Frontend | **No streaming metrics display** — Plan specifies "Local HTTP (on a second internal port, e.g. 4142): for metrics/log streaming to the UI." Frontend implements no WebSocket, SSE, or EventSource connections. |
| DG-010 | Frontend | **`daily_requests_used` and `consecutive_errors` tracked but never displayed** — Fields exist in types but no UI component surfaces them. |
| DG-011 | Frontend | **No loading/error state when router status is unavailable** — `GroupStatusProvider` catches errors silently. Provider health cards show no entries until proxy starts, with no indication of why. |

---

## Accessibility Issues

| ID | File(s) | Description |
|---|---|---|
| ACC-001 | `Providers.tsx:488`, `ModelGroups.tsx:652`, `Onboarding.tsx:44` | Modals don't handle Escape key to close |
| ACC-002 | All modal components | Missing `role="dialog"`, `aria-modal`, `aria-label` attributes |
| ACC-003 | `Providers.tsx:544-559` | API key Clear/Show buttons lack `aria-label` |

---

## Summary by Severity

| Severity | Count |
|---|---|
| Critical | 5 |
| High | 8 |
| Medium | 30 |
| Low | 37 |
| **Total** | **80** |

## Summary by Category

| Category | Count |
|---|---|
| Backend Bugs | 30 |
| Frontend Bugs | 22 |
| Design Gaps | 11 |
| Build/Infrastructure | 8 |
| Accessibility | 3 |
| Test Coverage | 3 |
| Code Quality | 3 |

---

## Recommended Priority Order for Fixes

1. **FIND-001** — Add timeout wrapper around streaming body (`bytes_stream()`)
2. **FIND-002** — Propagate config changes from Tauri to sidecar (reload `AppState.groups`/`providers` after save/delete operations)
3. **FIND-003** — Initialize `daily_requests_used` from DB on startup
4. **FIND-004** — Use SIGTERM instead of SIGKILL for sidecar shutdown, await graceful exit
5. **FIND-005** — Record errors for failed streaming responses (call `on_complete` with error state)
6. **FIND-007** — Add `Authorization` header to model detail requests
7. **FIND-009/010/011** — Create `EntryState` for new entries when added via UI
8. **FIND-012/013** — Include `dailyRequestQuota` and `modelOverrides` in `ProviderModal` save
9. **FIND-014** — Reduce lock scope in `run_cooldown_check` or use read-copy-update pattern
10. **FIND-021** — Sanitize `AppError::InternalError` messages before exposing to client

---

## Notes

- 103 Rust tests pass (up from 102 in Audit 003, 86 in Audit 002, 76 in Audit 001). TypeScript check is clean.
- Frontend has type tests and store tests but zero component/page tests.
- The architectural gap between Tauri process config writes and sidecar proxy state (FIND-002) is the single most impactful remaining issue — it means any config change requires a proxy restart to take effect.
- The streaming timeout gap (FIND-001) means the latency failover trigger is ineffective for the most common use case (streaming requests).
- This is Audit 004 — subsequent audits should track remediation of these findings.
