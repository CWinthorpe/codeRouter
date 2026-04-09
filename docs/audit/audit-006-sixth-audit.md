# CodeRouter — Audit Report 006

**Date:** 2026-04-08  
**Scope:** Full project re-audit — bugs, design gaps, security, and completeness  
**Reference:** `docs/plan.md` (design spec), `docs/audit/audit-005-fifth-audit.md` (prior audit)  
**Build Status:** 106 Rust tests passing (12 types + 94 sidecar), TypeScript check clean

---

## Executive Summary

This is the sixth audit of CodeRouter. Test count has grown from 76 → 86 → 102 → 103 → 106. Many issues from prior audits have been addressed. **30 findings** remain across the Rust sidecar, React frontend, and project infrastructure.

**The most impactful remaining issues are:**

- **`daily_tokens_used` / `daily_requests_used` diverge across entries for the same provider** — `record_success` increments counters only for the specific entry that handled the request, not all entries for the same provider. After startup initialization loads provider-wide totals into all entries, they immediately diverge. `select_entry` can incorrectly skip one entry while routing to another for the same provider, even though the provider's actual total exceeds the quota.
- **`TimeoutStream` uses failover timeout for streaming body** — The `latency_timeout_ms` (default 30s) is meant as a time-to-first-byte failover threshold, but `TimeoutStream` applies it to the entire response body. A streaming response generating 2000 tokens at 50 tok/s would take ~40s and be cut off.
- **Tauri process cannot access sidecar's router state for OpenCode config injection** — `build_entry_statuses()` reads `router::get_global_router_state()` which is a process-local `OnceLock` set only in the sidecar process. In the Tauri process it always returns `None`, so all entries are treated as active, including those in cooldown or quota-exhausted.
- **Failed streaming responses recorded as "success"** — The `MetricsRecordingStream` callback always sets `status: "success"` and calls `router::record_success`, even when the stream errors. Partial tokens may be recorded as a successful request.
- **Sidecar binary naming mismatch** — `build.sh` copies the binary with a target-triple suffix but `tauri.conf.json` references it without one. Tauri v2 does not auto-append platform suffixes to `externalBin`.

---

## Previous Audit Fix Verification

| Audit 005 Issue | Status | Notes |
|---|---|---|
| FIND-001: Daily queries hardcode reset_hour=0 | ✅ Fixed | Now uses provider's `quota_reset_utc_hour` |
| FIND-002: Metrics task orphaned on shutdown | ✅ Fixed | Handle now stored and awaited |
| FIND-003: Model detail missing auth header | ✅ Fixed | Authorization header now added |
| FIND-004: Streaming latency measures TTFB | ✅ Fixed | Now measures full streaming duration |
| FIND-005: init_db() called twice | ❌ **NOT FIXED** — Still called in `ensure_first_run()` and `start_server()` (see FIND-005) |
| FIND-006: TimeoutStream semantic mismatch | ❌ **NOT FIXED** — Still uses `latency_timeout_ms` as inter-chunk gap timeout (see FIND-002) |
| FIND-007: Quota reset only processes QuotaExhausted | ✅ Fixed |
| FIND-008: Cooldown-expired entries keep Cooldown status | ✅ Fixed |
| FIND-009: OpenCode config writer uses stale status | ❌ **NOT FIXED** — Still uses persisted JSON string, not runtime state (see FIND-003) |
| FIND-010: Config reload loses non-counter state | ✅ Fixed |
| FIND-011: useGroupStatusPoll double interval | ✅ Fixed |
| FIND-012: Drag throttle skips preventDefault | ✅ Fixed |
| FIND-013: UsageMetrics never refetches on date range | ✅ Fixed |
| FIND-014: Config reload notification no retry | ⚠️ Partially — retry mechanism added but still limited |
| FIND-015: JSON parse errors misclassified | ✅ Fixed |
| FIND-016: No multi-modal support | ⚠️ Partially — basic array content handled but tool_use not supported |
| FIND-017: proxy_running race on spawn | ✅ Fixed |
| FIND-018: No Secret Service fallback | ⚠️ Partially — error message improved but no file-based fallback |
| FIND-019: Credential error leaked to client | ✅ Fixed |
| FIND-020: Refresher interval not reactive | ⚠️ Partially — now re-reads config periodically |
| FIND-021: Duplicate fetchSummaries | ✅ Fixed |
| FIND-022: Duplicate health polling | ✅ Fixed |
| FIND-023: Settings reset doesn't clear store | ⚠️ Partially — providers/groups cleared but `loadError`, `healthData`, `proxyStatus`, `recentStreamRequests` not reset (see FIND-016) |
| FIND-024: Drag-and-drop keys unstable | ⚠️ Partially — improved but `entryKeys` and `entries` state updates not atomic (see FIND-019) |
| FIND-025: Timezone mismatch in date inputs | ⚠️ Partially — improved but local vs UTC date boundaries still differ (see FIND-021) |
| FIND-026: SSE re-emit may include \r | ✅ Fixed |
| FIND-027: Searchable dropdown not implemented | ✅ Fixed — custom `SearchableSelect` implemented |
| FIND-028: System agents shown in UI | ✅ Fixed |
| FIND-029: shadcn/ui unused | ✅ Removed — custom implementations are acceptable |
| FIND-030: Dead IPC functions | ✅ Removed — not a functional bug |
| FIND-031: Edit modal loses API key context | ⚠️ Partially — UI hint added but no visual indicator of preserved key |
| FIND-032: Modals no keyboard support | ⚠️ Partially — Escape key added but focus trapping missing |
| FIND-033: Hardcoded x86_64 sidecar path | ⚠️ Partially — now uses `sidecar_target_suffix()` but naming mismatch with build.sh (see FIND-013) |
| FIND-034: clsx missing from dependencies | ✅ Fixed |
| FIND-035: recharts v3 suspicious | ✅ Verified — version exists on npm |
| FIND-036: build.sh always runs npm install | ⚠️ Partially — now checks for node_modules but still runs install |

---

## Critical Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-001 | Critical | `sidecar/src/proxy/router.rs:276-307`, `sidecar/src/proxy/server.rs:457-459` | **`daily_tokens_used` / `daily_requests_used` diverge across entries for same provider** — `record_success` increments counters only for the specific entry that handled the request. `init_daily_totals_from_db` correctly loads provider-wide totals into ALL entries at startup, but after that, entries for the same provider diverge. If provider P has entries E1 and E2, and E1 handles requests all day, E1's counter approaches the quota while E2's stays low. `select_entry` incorrectly skips E1 but still routes to E2 even though the provider's actual total exceeds the quota. |
| FIND-002 | Critical | `sidecar/src/proxy/server.rs:618-623` | **`TimeoutStream` uses failover timeout for streaming body** — `latency_timeout_ms` (default 30s) is the time-to-first-byte failover threshold per the plan. But `TimeoutStream` applies this same 30s timeout to the entire response body reading. A streaming response generating 2000 tokens at 50 tok/s would take ~40s and be cut off. The plan's latency timeout is about failover when a provider "does not respond" — meaning first byte, not last byte. |
| FIND-003 | Critical | `src-tauri/src/commands.rs:446-455`, `sidecar/src/opencode/config_writer.rs:111-126` | **Tauri process cannot access sidecar's router state for OpenCode config injection** — `build_entry_statuses()` reads `router::get_global_router_state()` which is a process-local `OnceLock` set only in the sidecar process. In the Tauri process it always returns `None`, so `entry_statuses` is always empty. In `inject_provider`, the check `.unwrap_or(true)` treats all entries as active. OpenCode config injection includes models from providers that are in cooldown, quota-exhausted, or otherwise unavailable. |
| FIND-004 | Critical | `sidecar/src/proxy/server.rs:430-460`, `776-785` | **Failed streaming responses recorded as "success"** — `MetricsRecordingStream::poll_next` calls the completion callback on both success (`Poll::Ready(None)`) and error (`Poll::Ready(Some(Err(e)))`). The callback always sets `status: "success"` and calls `router::record_success`. If the stream fails (timeout, network error), partial tokens may have been accumulated and are recorded as a successful request. The `tokens_used > 0` guard prevents `record_success` from running when zero tokens were produced, but the metrics event is still recorded as `"success"`. |

## High-Severity Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-005 | High | `sidecar/src/main.rs:23`, `sidecar/src/proxy/server.rs:76` | **`init_db()` called twice in sidecar process** — `ensure_first_run()` calls `init_db()` at main.rs:23, then `start_server()` calls it again at server.rs:76. Both open separate SQLite connections. The second call is redundant (migrations are idempotent) but wastes resources. |
| FIND-006 | High | `sidecar/src/metrics/queries.rs:142` | **`get_usage_by_day` groups by UTC midnight instead of provider reset hour** — The `start_ts` filter correctly uses `reset_hour`, but the `GROUP BY` uses SQLite's `DATE(ts, 'unixepoch')` which groups by UTC midnight. For a provider with `quota_reset_utc_hour=6`, a request at 05:00 UTC belongs to the previous "day" but gets grouped into the UTC-midnight day boundary. |
| FIND-007 | High | `sidecar/src/metrics/queries.rs:160-189` | **`get_usage_by_group` ignores provider reset hour entirely** — This function takes only `days` as a parameter, no `reset_hour`. The `start_ts` is calculated as `now - days` from the current moment, not from any provider's reset hour. Daily group usage totals don't align with any provider's quota reset boundary. |
| FIND-008 | High | `sidecar/src/proxy/upstream.rs:74-97` | **Non-streaming response body reading has no timeout** — `send_with_timeout` applies the timeout via `req.timeout()` which only covers the `send()` call (getting response headers). The response body is read later via `resp.bytes().await` with no timeout. A hung connection during body reading would block indefinitely (until the client's 120s timeout). |
| FIND-009 | High | `sidecar/src/proxy/server.rs:1008-1028` | **`handle_internal_config_reload` doesn't remove stale router entries** — The config reload preserves existing entries and inserts new ones, but never removes entries that were deleted from the config. Stale entries accumulate in `router_state.entries` over time. |
| FIND-010 | High | `src-tauri/src/commands.rs:32-49`, `66-86` | **`daily_request_quota` not exposed in `ProviderResponse`** — The `Provider` model has `daily_request_quota`, but `ProviderResponse` omits it entirely. The `From<&Provider>` impl doesn't include it. The frontend cannot see or configure daily request quotas. |
| FIND-011 | High | `src/hooks/useProxyStatusPoll.ts:11` | **Health URL uses `localhost` fallback instead of `127.0.0.1`** — When `appConfig` is null, `proxyHost` defaults to `'localhost'`. But the Rust backend default is `'127.0.0.1'`. On some Linux systems, `localhost` resolves to IPv6 `::1` while the proxy binds to `127.0.0.1` only (IPv4), causing health checks to fail. |
| FIND-012 | High | `src/pages/Providers.tsx:424-444` | **ProviderModal form state may not reset between add/edit** — All form state is initialized from `provider` prop on first render. If the user opens the modal to edit provider A, closes it, then opens it to add a new provider, React may reuse the component instance and show provider A's data. Should add a `key` prop. |
| FIND-013 | High | `build.sh:33`, `src-tauri/tauri.conf.json:37` | **Sidecar binary naming mismatch** — `build.sh` copies the binary with a target-triple suffix (e.g., `coderouter-proxy-x86_64-unknown-linux-gnu`) but `tauri.conf.json` references it without one (`"sidecar/coderouter-proxy"`). Tauri v2 does not auto-append platform suffixes to `externalBin`. The AppImage build will fail to find the sidecar. |

## Medium-Severity Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-014 | Medium | `src/pages/Settings.tsx:119-127` | **Settings reset doesn't clear all store state** — After `resetAllConfig()`, `setProviders([])` and `setGroups([])` are called, but `loadError`, `healthData`, `proxyStatus`, and `recentStreamRequests` are not reset. `proxyStatus` remains `'running'` and `recentStreamRequests` retains stale data. |
| FIND-015 | Medium | `src/pages/ModelGroups.tsx:618-619, 768` | **Drag-and-drop `entryKeys` and `entries` state updates not atomic** — `setEntries` and `setEntryKeys` are called separately. React may batch them, but there's no guarantee they update together. `key={entryKeys[idx]}` can reference `undefined` if `entryKeys` is shorter than `entries`. |
| FIND-016 | Medium | `src/pages/ModelGroups.tsx:518-523` | **`SearchableSelect` loses selected value when dropdown closes** — When the user focuses the input again, `onFocus` sets `setSearch('')`, clearing the visible text. If the user clicks away without selecting anything, the input becomes blank even though a value was previously selected. |
| FIND-017 | Medium | `src/pages/UsageMetrics.tsx:38-42, 198` | **Date range filtering uses local timezone, backend uses UTC** — `formatDate` uses `getFullYear()`, `getMonth()`, `getDate()` (local timezone). `startOfDay(new Date(customStart + 'T00:00:00'))` parses as local time. The backend uses UTC for daily boundaries via `quota_reset_utc_hour`. Off-by-one-day possible for non-UTC users. |
| FIND-018 | Medium | `src/types/index.ts:29` | **`modelOverrides` saved but never returned from backend** — TypeScript `Provider` interface has `modelOverrides?: ProviderModel[]`. The Rust `ProviderResponse` struct does not include this field. Overrides are saved to the Rust side but never returned back to the frontend. |
| FIND-019 | Medium | `src/pages/Dashboard.tsx:48-51` | **`quota_exhausted` status not counted in provider health** — `getEntryStatusCounts` only counts `active`, `cooldown`, and `manually_disabled`. `quota_exhausted` entries are silently ignored, making providers appear healthier than they are. |
| FIND-020 | Medium | `src/pages/Providers.tsx:398-404` | **Model browser "Add to group" button is disabled** — The button is `disabled` with title "Coming soon". The plan says the model browser should allow adding models to groups. |
| FIND-021 | Medium | `src-tauri/src/commands.rs:595-627` | **AppImage sidecar path uses architecture-specific suffix that may not match build output** — `sidecar_target_suffix()` produces `coderouter-proxy-{arch}-{os}-{family}` (e.g., `coderouter-proxy-x86_64-linux-unix`). The AppImage path expects this exact suffix under `usr/bin/sidecar/`. If the build pipeline produces a different naming convention, the sidecar won't be found. |
| FIND-022 | Medium | `sidecar/src/proxy/server.rs:1030-1037` | **`handle_internal_config_reload` opens a third DB connection** — Opens a new `init_db()` connection just to reload daily totals, then drops it. Wasteful. |

## Low-Severity Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-023 | Low | `sidecar/src/metrics/recorder.rs:22-30` | `calculate_cost` returns 0.0 for partial pricing (only input or only output cost set) |

---

## Design Gaps vs plan.md

| ID | Area | Description |
|---|---|---|
| DG-001 | Backend | **`daily_request_quota` not enforced** — Field exists in struct but not enforced in routing. |
| DG-002 | Backend | **Latency timeout cooldown not configurable** — Plan: "Cooldown period (configurable, default 5 minutes)." Hardcoded. |
| DG-003 | Backend | **Consecutive errors cooldown not configurable** — Plan: "Fixed cooldown period (configurable, default 10 minutes)." Hardcoded. |
| DG-005 | Backend | **`opencode.json` cached config not implemented** — Plan lists `~/.config/coderouter/opencode.json`. Not read or written. |
| DG-006 | Backend | **System agents `compaction`, `title`, `summary` missing** — Plan lists 7 agents. `AgentMapping` only has `build`, `plan`, `general`, `explore`, `small_model`. |
| DG-007 | Frontend | **No streaming metrics display** — Plan specifies "Local HTTP (on a second internal port, e.g. 4142): for metrics/log streaming to the UI." Frontend implements EventSource but no real-time streaming token visualization. |
| DG-008 | Frontend | **No 404/not-found route** — Unknown paths show blank/error. |
| DG-009 | Frontend | **Model browser "Add to group" not implemented** — Button is disabled with "Coming soon". |

---

## Summary by Severity

| Severity | Count |
|---|---|
| Critical | 4 |
| High | 9 |
| Medium | 9 |
| Low | 1 |
| **Total** | **23** |

## Summary by Category

| Category | Count |
|---|---|
| Backend Bugs | 10 |
| Frontend Bugs | 8 |
| Design Gaps | 8 |
| Build/Infrastructure | 2 |

---

## Recommended Priority Order for Fixes

1. **FIND-001** — `record_success` must increment counters for ALL entries belonging to the same `provider_id`, not just the specific entry
2. **FIND-002** — Use a much longer timeout for streaming body (e.g., 120s+) or remove `TimeoutStream` and rely on HTTP client's 120s connection timeout
3. **FIND-003** — Fetch router status from the sidecar's `/internal/router/status` HTTP endpoint instead of reading the global static `OnceLock`
4. **FIND-004** — The completion callback needs to know whether the stream completed successfully or errored, and record the appropriate status
5. **FIND-005** — Remove the `init_db()` call from `ensure_first_run()` since `start_server()` needs its own connection anyway
6. **FIND-006/007** — Use custom date calculation in `GROUP BY` that accounts for reset hour: `DATE(ts - reset_hour * 3600, 'unixepoch')`
7. **FIND-008** — Wrap `resp.bytes().await` calls in `tokio::time::timeout`
8. **FIND-009** — After merging config reload, remove entries from `router_state.entries` that no longer exist in the new config
9. **FIND-010** — Add `daily_request_quota: Option<u64>` to `ProviderResponse` and the `From` impl
10. **FIND-013** — Fix sidecar binary naming: either remove suffix from `build.sh` or update `tauri.conf.json` to match
11. **FIND-014** — Reset `loadError`, `healthData`, `proxyStatus`, and `recentStreamRequests` on settings reset
12. **FIND-017** — Use UTC-based date inputs or convert local dates to UTC before sending to backend
13. **FIND-018** — Add `model_overrides` to Rust `ProviderResponse` and the `From` impl
14. **FIND-022** — Reuse existing DB connection in `handle_internal_config_reload` instead of opening a third one

---

## Notes

- 106 Rust tests pass (up from 103 in Audit 005, 102 in Audit 004, 86 in Audit 003, 76 in Audit 001). TypeScript check is clean.
- 7 frontend tests exist (type tests + store tests). Zero component/page tests.
- The counter divergence bug (FIND-001) is the most impactful remaining data correctness issue — it means per-provider quota enforcement becomes unreliable as soon as a provider has entries in multiple groups.
- The `TimeoutStream` cutting off long streaming responses (FIND-002) directly impacts the primary use case of AI coding assistants that generate large code completions.
- This is Audit 006 — subsequent audits should track remediation of these findings.
