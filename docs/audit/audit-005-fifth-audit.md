# CodeRouter — Audit Report 005

**Date:** 2026-04-08  
**Scope:** Full project re-audit — bugs, design gaps, security, and completeness  
**Reference:** `docs/plan.md` (design spec), `docs/audit/audit-004-fourth-audit.md` (prior audit)  
**Build Status:** 103 Rust tests passing (12 types + 91 sidecar), TypeScript check clean

---

## Executive Summary

This is the fifth audit of CodeRouter. Test count: 103 Rust tests passing, TypeScript clean. Many issues from prior audits have been addressed, but **48 findings** remain. Several are newly discovered; others were only partially fixed in prior rounds.

**The most impactful remaining issues are:**

- **`get_daily_summary` / `get_usage_by_day` / `get_latency_percentiles` hardcode `reset_hour = 0`** — These three query functions ignore the provider's configured `quota_reset_utc_hour`, always using midnight UTC. Daily summaries, usage charts, and latency percentiles will be wrong for any provider with a non-zero reset hour.
- **Metrics background task orphaned on shutdown** — The `_metrics_handle` `JoinHandle` is immediately dropped, detaching the task. Pending metrics writes can be lost when the process exits.
- **Model detail requests have no authentication header** — The `/v1/models/{model_id}` detail fetch sends no `Authorization` header. Most providers require auth, so model metadata (context window, pricing) will fail to be fetched.
- **Streaming latency measures TTFB, not full duration** — Latency is captured immediately after `process_response` returns for streaming, which is essentially time-to-first-byte. The full streaming duration (which can be many seconds) is never measured.
- **`clsx` missing from `package.json`** — `src/lib/utils.ts` imports `clsx` but it's not listed as a dependency. It only exists as a transitive dependency. If the transitive dep is removed, the build breaks.
- **Drag throttle skips `preventDefault`, can cancel drag** — During the 50ms throttle window, `e.preventDefault()` is not called, which can cause the browser to cancel the drag operation.
- **Request data never refetches on date range change** — UsageMetrics fetches 1000 requests once on mount. Switching to "Last 30 days" or a custom range only filters the already-fetched data client-side. Older data is never fetched.

---

## Previous Audit Fix Verification

| Audit 004 Issue | Status | Notes |
|---|---|---|
| FIND-001: Streaming body no timeout | ⚠️ Partially — `TimeoutStream` added but uses request timeout as inter-chunk gap timeout, which is semantically different from the plan's intent (see FIND-006) |
| FIND-002: Config changes not visible to sidecar | ⚠️ Partially — notification mechanism added but has no retry if sidecar is down (see FIND-014) |
| FIND-003: daily_requests_used not initialized | ✅ Fixed |
| FIND-004: kill_sidecar sends SIGKILL | ⚠️ Partially — now uses SIGTERM but graceful shutdown still uses fixed 500ms sleep (see FIND-019) |
| FIND-005: Failed streaming responses not recorded | ✅ Fixed |
| FIND-006: MetricsRecordingStream callback (0,0) | ✅ Fixed |
| FIND-007: Model detail no auth header | ❌ **NOT FIXED** — Still missing Authorization header (see FIND-003) |
| FIND-008: extract_from_raw_json dead code | ✅ Fixed |
| FIND-009: select_entry doesn't create EntryState for new entries | ✅ Fixed |
| FIND-010: handle_internal_router_set_entry doesn't create EntryState | ✅ Fixed |
| FIND-011: set_entry_enabled fails for new entries | ✅ Fixed |
| FIND-012/013: ProviderModal loses dailyRequestQuota/modelOverrides | ✅ Fixed |
| FIND-014: run_cooldown_check holds lock | ⚠️ Partially — scope reduced but still held for iteration (see FIND-013) |
| FIND-015: process_response reads error body for streaming | ✅ Fixed |
| FIND-016: record_success doesn't reset cooldown_duration_seconds | ✅ Fixed |
| FIND-017: record_quota_exhausted doesn't reset consecutive_errors | ✅ Fixed |
| FIND-018: SSE parsing only splits on \n | ⚠️ Partially — `\r` stripping added but re-emit may still include stale `\r` (see FIND-026) |
| FIND-019: init_daily_totals_from_db double-counts | ⚠️ Partially — documented behavior, still semantically confusing |
| FIND-020: init_db() called twice | ❌ **NOT FIXED** — Still called twice at startup (see FIND-005) |
| FIND-021: AppError::InternalError leaks details | ✅ Fixed |
| FIND-022: handle_models uses current timestamp for created | ✅ Fixed |
| FIND-023: ProviderResponse missing daily_request_quota | ✅ Fixed |
| FIND-024: atomic_write leaves temp files on failure | ✅ Fixed |
| FIND-025: Daily queries use midnight UTC | ❌ **NOT FIXED** — `get_daily_summary`, `get_usage_by_day`, `get_latency_percentiles` still hardcode `reset_hour = 0` (see FIND-001) |
| FIND-026: Model details fetched sequentially | ⚠️ Partially — timeout added but still sequential |
| FIND-027: Drag-and-drop key collision | ❌ **NOT FIXED** — Keys still unstable during drag (see FIND-008) |
| FIND-028: Timezone mismatch in date inputs | ❌ **NOT FIXED** — Still present (see FIND-009) |
| FIND-029: handleResetSettings doesn't clear store | ❌ **NOT FIXED** — Still present (see FIND-004 frontend) |
| FIND-030: CSP wildcard port | ⚠️ Partially — `localhost:*` restricted but `127.0.0.1:*` still wildcard |
| FIND-031: withGlobalTauri | ✅ Fixed — now `false` |
| FIND-032: Hardcoded pkg-config path | ⚠️ Partially — still hardcoded in Makefile |
| FIND-033: No npm install in build.sh | ❌ **NOT FIXED** — build.sh still assumes node_modules exist |
| FIND-034: OpenOptions without truncate | ⚠️ Partially — `set_len(0)` called before write but read happens before truncate |
| FIND-035: shadcn/ui barely used | ❌ **NOT FIXED** — Still barely used |
| FIND-036: quota_exhausted not counted in Dashboard | ⚠️ Partially — status display added but counting logic incomplete |
| FIND-037: Frontend test coverage minimal | ⚠️ Partially — 7 tests exist but no component tests |
| FIND-038: IPC commands not wired | ❌ **NOT FIXED** — Still dead code (see FIND-011) |
| FIND-039: No toast on toggle | ❌ **NOT FIXED** — Still no feedback |
| FIND-040: LiveStatusPanel fallback doesn't account for quota_exhausted | ⚠️ Partially |
| FIND-041: ProxyStatusCard shows "Stopped" for unknown | ⚠️ Partially |
| FIND-042: poll_health runs forever | ⚠️ Partially — cancellation mechanism added but not fully wired |
| FIND-043: MetricsRecorder handle not awaited | ❌ **NOT FIXED** — Handle still dropped immediately (see FIND-002) |

---

## Critical Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-001 | Critical | `sidecar/src/metrics/queries.rs:53, 125, 202` | **Daily queries hardcode `reset_hour = 0`** — `get_daily_summary`, `get_usage_by_day`, and `get_latency_percentiles` all use `let reset_hour = 0u32`, ignoring the provider's configured `quota_reset_utc_hour`. Only `get_today_token_totals` uses the correct reset hour. Daily summaries, usage charts, and latency percentiles will be wrong for any provider with a non-zero reset hour. |
| FIND-002 | Critical | `sidecar/src/proxy/server.rs:76, 153` | **Metrics background task orphaned on shutdown** — `_metrics_handle` `JoinHandle` is immediately dropped at line 76, detaching the spawned task. On shutdown, the `MetricsRecorder` Arc is dropped which closes the channel, but the handle is never `.await`ed — no guarantee pending metrics are flushed before process exits. |
| FIND-003 | Critical | `sidecar/src/models/refresher.rs:99` | **Model detail request missing authentication header** — The `/v1/models/{model_id}` detail request is sent without any `Authorization` header. Most OpenAI-compatible providers require authentication on this endpoint, so model metadata (context window, pricing) will fail to be fetched for authenticated providers. |
| FIND-004 | Critical | `sidecar/src/proxy/server.rs:397` | **Streaming latency measures TTFB, not full duration** — `start.elapsed().as_millis()` is captured immediately after `process_response` returns for streaming responses, which is essentially time-to-first-byte (connection + first chunk). The full streaming duration (which can be many seconds) is never measured. Latency metrics for streaming requests are misleadingly low. |
| FIND-005 | Critical | `sidecar/src/proxy/server.rs:73, 75` | **`init_db()` called twice at startup** — `init_daily_totals_from_db` internally calls `init_db()` to open its own SQLite connection, then `metrics_db::init_db()` is called again at line 75 for the metrics recorder. Two separate connections to the same file, wasteful and potentially inconsistent. |

## High-Severity Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-006 | High | `sidecar/src/proxy/server.rs:575-711` | **`TimeoutStream` uses request timeout as inter-chunk gap timeout** — The `TimeoutStream` wraps `resp.bytes_stream()` and fires when `last_chunk.elapsed() > timeout`. But the timeout value comes from `latency_timeout_ms` (default 30s), which the plan intends as a **total request timeout**. A slow stream sending one token every 29 seconds would never timeout, but one that pauses for 31 seconds would. Semantically different from the plan's intent. |
| FIND-007 | High | `sidecar/src/metrics/scheduler.rs:119` | **Quota reset only processes `QuotaExhausted` entries** — `run_quota_reset` only matches entries where `s.status == EntryStatus::QuotaExhausted`. If `on_quota_exhausted` is `false` in the failover config, the status is never set to `QuotaExhausted`, so `run_quota_reset` never resets `daily_tokens_used` and `daily_requests_used`. Token counters accumulate forever if the trigger is disabled. |
| FIND-008 | High | `sidecar/src/proxy/router.rs:168-175` | **Cooldown-expired entries returned with `Cooldown` status** — When a cooldown has expired, `select_entry` returns `true` allowing the entry to be selected, but its `status` field remains `Cooldown`. The status is only transitioned to `Active` on the next successful request. The health endpoint shows stale status between cooldown expiry and the next successful request. |
| FIND-009 | High | `sidecar/src/opencode/config_writer.rs:109-111` | **OpenCode config writer uses persisted `status` string, not runtime state** — `inject_provider` and `preview_opencode_config` filter entries by `e.status == "active"` (the persisted JSON string, which defaults to `"active"` and is never updated by the runtime). The runtime tracks status in `EntryState` (router state HashMap), not in the `GroupEntry` struct. The wrong provider's model metadata may be injected into OpenCode config. |
| FIND-010 | High | `sidecar/src/proxy/server.rs:943-966` | **Config reload loses non-counter state** — When config is reloaded, a fresh `RouterState` is created. Only `daily_tokens_used` and `daily_requests_used` are preserved. `consecutive_errors`, `cooldown_until`, `cooldown_duration_seconds`, and `status` (except for manually disabled entries) are all lost. A provider in cooldown will suddenly become active after a config change. |
| FIND-011 | High | `src/hooks/useGroupStatusPoll.tsx:46` | **Wrong useEffect dependency causes double interval creation** — The effect depends on `[loading]`. On mount `loading=true`, effect runs and sets interval. After first poll, `loading` flips to `false`, triggering the effect to re-run — cleaning up the first interval and creating a second one. Should be `[]`. |
| FIND-012 | High | `src/pages/ModelGroups.tsx:595-609` | **Drag throttle skips `preventDefault`, can cancel drag** — `handleDragOver` returns early during the 50ms throttle window (`if (dragOverThrottleRef.current) return;`) BEFORE calling `e.preventDefault()`. During the throttle window, the browser doesn't get `preventDefault`, which can cause the drag operation to be cancelled. |
| FIND-013 | High | `src/pages/UsageMetrics.tsx:206-221` | **Request data never refetches on date range change** — The `useEffect` has `[]` dependency array. Data is fetched once on mount (1000 requests max). If user switches to "Last 30 days" or a custom range, the same 1000-request dataset is filtered client-side. Older data is never fetched. |
| FIND-014 | High | `src-tauri/src/commands.rs:97-102` | **Config reload notification has no retry if sidecar is down** — If the sidecar is restarting when the notification is sent, the config change is silently lost. The sidecar will continue using stale config until the next manual reload or restart. |

## Medium-Severity Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-015 | Medium | `sidecar/src/proxy/server.rs:603, 611` | **JSON parse errors misclassified as `Network`** — A JSON parsing error from the upstream response is classified as `RequestError::Network`, which increments the consecutive error counter. This is semantically incorrect — it should be `ServerError`. Could cause false failovers if a provider returns slightly malformed JSON. |
| FIND-016 | Medium | `sidecar/src/proxy/translator.rs:11-15` | **`AnthropicMessage` doesn't support multi-modal content arrays** — Only supports `content: String`. Anthropic's API supports content arrays for images. Multi-modal requests will be incorrectly serialized (content array JSON-serialized as a string). |
| FIND-017 | Medium | `src-tauri/src/main.rs:224-230` | **`proxy_running` not set to `true` after spawn** — Sidecar is spawned but `proxy_running` is not set to `true` in `setup()`. The `poll_health` task will eventually set it when the health check succeeds, but there's a race window where the sidecar is running but `proxy_running` is `false`. Tray icon shows "inactive" for up to 5 seconds. |
| FIND-018 | Medium | `sidecar/src/credentials/keychain.rs:10` | **No fallback when Secret Service daemon unavailable** — `SecretService::connect(EncryptionType::Dh)` will fail on systems without a running secret service daemon (minimal WM, headless). No fallback mechanism. |
| FIND-019 | Medium | `sidecar/src/proxy/server.rs:337-340` | **Credential-not-found error leaked to client** — If the credential is not found in the keychain, the error message "Credential not found" is returned as an HTTP 500 to the client, leaking internal state information. |
| FIND-020 | Medium | `sidecar/src/proxy/server.rs:108-111` | **Refresher interval captured at startup** — `refresh_interval_hours` is read once at startup. If the user changes it in settings, the running refresher task won't pick up the new value until restart. |
| FIND-021 | Medium | `src/pages/Dashboard.tsx:280-287` | **Duplicate `fetchSummaries` calls on mount** — Two separate `useEffect` hooks both call `fetchSummaries` on mount with `[providers]` as dependency. Results in 2x simultaneous IPC calls. |
| FIND-022 | Medium | `src/pages/Dashboard.tsx:12` + `src/hooks/useProxyStatusPoll.ts:4` | **Duplicate health polling** — Both `useHealthPoll` (Dashboard) and `useProxyStatusPoll` (hook) poll the same `/health` endpoint every 5 seconds. The health endpoint gets hit twice per interval. |
| FIND-023 | Medium | `src/pages/Settings.tsx:114-130` | **Settings reset does NOT clear Zustand store** — `handleResetSettings` calls `resetAllConfig()` IPC and refreshes `appConfig`, but never calls `setProviders([])` or `setGroups([])`. The store retains stale providers/groups in memory until full page reload. |
| FIND-024 | Medium | `src/pages/ModelGroups.tsx:716` | **Drag-and-drop keys are unstable** — Key is `${entry.providerId}-${entry.modelId}-${idx}-${entry.priority}`. Both `idx` and `priority` change during drag reorder, causing React to unmount/remount DOM nodes instead of reordering. Causes visual flicker and breaks reconciliation. If two entries share the same provider+model, keys collide. |
| FIND-025 | Medium | `src/pages/UsageMetrics.tsx:174-175, 202` | **Timezone mismatch in custom date range** — `formatDate` uses `toISOString().slice(0, 10)` (UTC) but `<input type="date">` works in local timezone. `new Date(customStart + 'T00:00:00')` parses as local time, then `startOfDay` converts to UTC. Off-by-one-day possible for non-UTC users. |
| FIND-026 | Medium | `sidecar/src/proxy/translator.rs:544-566` | **SSE re-emit may include stale `\r` characters** — The tracker strips `\r` for parsing but emits the original `line_bytes` which may still contain `\r`. Could cause SSE parsing issues for the client. |
| FIND-027 | Medium | `src/pages/ModelGroups.tsx:777-807` | **Searchable dropdown not implemented** — Plan specifies "searchable dropdown of providers and their models." Plain `<select>` elements are used. User cannot type to filter providers or models. |
| FIND-028 | Medium | `src/pages/OpenCodeSetup.tsx:37` | **Agent dropdowns show system agents** — `AGENT_KEYS` includes `compaction`, `title`, `summary` which the plan marks as "system (hidden)." The UI shows all 7 plus small_model. |
| FIND-029 | Medium | `src/components/ui/` (8 components) | **shadcn/ui components defined but unused** — `Dialog`, `Select`, `Tabs`, `Progress` are scaffolded but never used. All pages use custom implementations. |
| FIND-030 | Medium | `src/lib/ipc.ts:154-163` | **Dead IPC functions** — `getUsageByDay`, `getUsageByGroup`, and `getLatencyPercentiles` are defined but never imported or called. UsageMetrics computes charts client-side from raw data, inefficient for large datasets. |
| FIND-031 | Medium | `src/pages/Providers.tsx:419` | **Edit modal loses API key context** — When editing, `apiKey` initializes to `''`. The form passes empty string to `saveProvider`. Relies entirely on backend to interpret empty as "don't update keychain." No visual feedback about whether existing key is preserved. |
| FIND-032 | Medium | `src/components/Onboarding.tsx:44`, `Providers.tsx:488`, `ModelGroups.tsx:652` | **Modals have no keyboard support** — No `role="dialog"`, no `aria-modal="true"`, no Escape key handler. Overlay dismissible by click only. |
| FIND-033 | Medium | `src-tauri/src/commands.rs:559, 574` | **Hardcoded x86_64 architecture in sidecar path** — Sidecar binary path is hardcoded to `coderouter-proxy-x86_64-unknown-linux-gnu`. On aarch64 or other architectures, this will fail. |
| FIND-034 | Medium | `src/lib/utils.ts:1` | **`clsx` missing from `package.json`** — `utils.ts` imports `clsx` but it's not listed as a dependency. Only exists as a transitive dependency. If the transitive dep is removed, the build breaks. |
| FIND-035 | Medium | `package.json:30` | **`recharts` v3 likely non-existent** — `"recharts": "^3.8.1"` — recharts is currently at v2.x. v3.8.1 may not exist on npm. Could be a typo for `^2.8.1`. |
| FIND-036 | Medium | `build.sh:48` | **`build.sh` always runs `npm install`** — Runs unconditionally on every build, even if dependencies haven't changed. Slows down rebuilds. |

## Low-Severity Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-037 | Low | `sidecar/src/proxy/server.rs:151-154` | Graceful shutdown uses fixed 500ms sleep instead of awaiting tasks |
| FIND-038 | Low | `sidecar/src/proxy/server.rs:91` | `_scheduler_handle` dropped, task detached |
| FIND-039 | Low | `sidecar/src/models/refresher.rs:52-56` | Anthropic models hardcoded, not fetched from API |
| FIND-040 | Low | `sidecar/src/proxy/ssrf.rs:44-59` | No DNS resolution check for domain hostnames (DNS rebinding possible) |
| FIND-041 | Low | `sidecar/src/proxy/server.rs:196-206` | `/v1/models` doesn't check `daily_request_quota` |
| FIND-042 | Low | `sidecar/src/proxy/server.rs:452, 419` | Metrics events silently dropped when channel full |
| FIND-043 | Low | `src-tauri/src/commands.rs:307-311` | `refresh_provider_models` loads providers twice (race condition) |
| FIND-044 | Low | `src/store/index.ts:30-61` | `loadInitialData` does redundant `loadError: null` sets — three separate state updates instead of one |
| FIND-045 | Low | `src/pages/UsageMetrics.tsx:648`, `Dashboard.tsx:121` | `StatusBadge` import at bottom of file (style violation) |
| FIND-046 | Low | `src/pages/ModelGroups.tsx:97` | Misleading `_tick` parameter in `cooldownCountdown` |
| FIND-047 | Low | `src/pages/OpenCodeSetup.tsx:128` | Unnecessary `groups` dependency in preview useEffect |
| FIND-048 | Low | `src/pages/Providers.tsx:311-319`, `ModelGroups.tsx:474-482` | Toggle switches missing accessible labels |

---

## Design Gaps vs plan.md

| ID | Area | Description |
|---|---|---|
| DG-001 | Backend | **`daily_request_quota` not enforced** — Field exists in struct but not enforced in routing or displayed in `/v1/models`. |
| DG-002 | Backend | **Latency timeout cooldown not configurable** — Plan: "Cooldown period (configurable, default 5 minutes)." Hardcoded. |
| DG-003 | Backend | **Consecutive errors cooldown not configurable** — Plan: "Fixed cooldown period (configurable, default 10 minutes)." Hardcoded. |
| DG-004 | Backend | **`/v1/completions` not "forwarded as-is"** — Plan: "Legacy completions endpoint (forwarded as-is if provider supports it)." Implementation translates to Anthropic messages format. |
| DG-005 | Backend | **`opencode.json` cached config not implemented** — Plan lists `~/.config/coderouter/opencode.json`. Not read or written. |
| DG-006 | Backend | **System agents `compaction`, `title`, `summary` missing** — Plan lists 7 agents. `AgentMapping` only has `build`, `plan`, `general`, `explore`, `small_model`. |
| DG-007 | Frontend | **No streaming metrics display** — Plan specifies "Local HTTP (on a second internal port, e.g. 4142): for metrics/log streaming to the UI." No WebSocket, SSE, or EventSource connections. |
| DG-008 | Frontend | **No 404/not-found route** — Unknown paths show blank/error. |
| DG-009 | Frontend | **`modelOverrides` type field is dead** — Defined in TypeScript types but never read or written anywhere in the codebase. |

---

## Summary by Severity

| Severity | Count |
|---|---|
| Critical | 5 |
| High | 9 |
| Medium | 22 |
| Low | 12 |
| **Total** | **48** |

## Summary by Category

| Category | Count |
|---|---|
| Backend Bugs | 18 |
| Frontend Bugs | 15 |
| Design Gaps | 9 |
| Build/Infrastructure | 4 |
| Accessibility | 2 |

---

## Recommended Priority Order for Fixes

1. **FIND-001** — Pass provider's `quota_reset_utc_hour` to `get_daily_summary`, `get_usage_by_day`, `get_latency_percentiles`
2. **FIND-002** — Store `metrics_handle` and `.await` it on shutdown to flush pending metrics
3. **FIND-003** — Add `Authorization` header to model detail requests
4. **FIND-004** — Measure full streaming duration, not just TTFB
5. **FIND-005** — Remove redundant `init_db()` call
6. **FIND-007** — Make `run_quota_reset` also reset counters for entries that exceeded quota but have `on_quota_exhausted` disabled
7. **FIND-008** — Transition `Cooldown` → `Active` when cooldown expires, not deferred to next request
8. **FIND-009** — Use runtime `EntryState` status in OpenCode config writer, not persisted JSON string
9. **FIND-010** — Preserve `consecutive_errors`, `cooldown_until`, `cooldown_duration_seconds` across config reload
10. **FIND-011** — Fix `useGroupStatusPoll` useEffect dependency from `[loading]` to `[]`
11. **FIND-012** — Call `e.preventDefault()` before throttle check in `handleDragOver`
12. **FIND-013** — Refetch request data when date range changes
13. **FIND-023** — Clear Zustand store providers/groups on settings reset
14. **FIND-034** — Add `clsx` to `package.json` dependencies
15. **FIND-035** — Verify `recharts` version (likely should be `^2.8.1`)

---

## Notes

- 103 Rust tests pass. TypeScript check is clean.
- 7 frontend tests exist (type tests + store tests). Zero component/page tests.
- The daily query hardcoded reset hour (FIND-001) is the most impactful remaining data correctness bug — any provider with a non-midnight reset hour will show wrong usage data.
- The missing auth header on model detail requests (FIND-003) means model metadata has likely never been successfully fetched for any authenticated provider.
- This is Audit 005 — subsequent audits should track remediation of these findings.
