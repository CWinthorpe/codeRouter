# CodeRouter — Audit Report 003

**Date:** 2026-04-08  
**Scope:** Full project re-audit — bugs, design gaps, security, and completeness  
**Reference:** `docs/plan.md` (design spec), `docs/audit/audit-002-post-fix-audit.md` (prior audit)  
**Build Status:** 102 Rust tests passing (12 lib + 90 sidecar), TypeScript check clean

---

## Executive Summary

This is the third audit of CodeRouter. Test count has grown from 76 → 86 → 102. Many issues from prior audits have been addressed, but **56 findings** remain across the Rust sidecar, React frontend, and project infrastructure.

**The most impactful remaining issues are:**

- **Streaming token counts are always zero** — Both Anthropic and OpenAI streaming paths return `(0, 0)` for token counts. Anthropic's lazy stream body hasn't been consumed when counts are read; OpenAI's passthrough doesn't attempt extraction. All streaming usage metrics, cost tracking, and quota enforcement are broken.
- **`QuotaExhausted` status is never set at runtime** — `process_response` never returns `RequestError::QuotaExhausted`, so the entire quota-exhausted failover trigger and recovery system is non-functional. Over-quota entries are skipped in routing but never automatically recovered without a restart.
- **In-memory groups go stale after `set_entry_enabled`** — The disk file is updated but `AppState.groups` is never refreshed, so subsequent model listings and routing use stale group definitions.
- **AppImage sidecar path detection is broken** — `APPIMAGE` env var points to the `.AppImage` file, not the FUSE mount point. The sidecar binary will not be found when running from a bundled AppImage.
- **shadcn/ui components are installed but barely used** — 8 components generated, only `Card`/`Button` used in one page. All other pages use custom implementations.

---

## Previous Audit Fix Verification

| Audit 002 Issue | Status | Notes |
|---|---|---|
| FIND-001: Router state in Tauri process | ✅ Fixed | Router state now initialized in Tauri process |
| FIND-002: Model refresher scheduling | ✅ Fixed | Recurring timer now implemented |
| FIND-003: Streaming SSE double-encoding | ✅ Fixed | SSE encoding corrected |
| FIND-003: Streaming token counts | ❌ **NOT FIXED** — Still always 0 (see FIND-001, FIND-002) |
| FIND-004: Cooldown → Active transition | ✅ Fixed | `record_success` now transitions status |
| FIND-005: 429 backoff starts at 60s | ✅ Fixed | First 429 now uses 60s baseline |
| FIND-006: Daily totals use provider reset hour | ✅ Fixed | Query now uses `quota_reset_utc_hour` |
| FIND-007: Timeout covers streaming body | ⚠️ Partially — timeout applied but probe has no shorter timeout (see FIND-009) |
| FIND-008: QuotaExhausted stale cooldown_until | ❌ **NOT FIXED** — QuotaExhausted system still non-functional (see FIND-003) |
| FIND-009: Stale router state after restart | ✅ Fixed | Router state reinitialized on restart |
| FIND-010: shadcn/ui theme vars | ⚠️ Partially — some vars mapped but components still barely used (see FIND-018) |
| FIND-011: handle_models runtime status | ✅ Fixed | Now checks runtime EntryStatus |
| FIND-012: Panic on invalid reset hour | ✅ Fixed | Validation added |
| FIND-013: 429 non-SSE streaming | ⚠️ Partially — status code check added but body handling incomplete (see FIND-014) |
| FIND-014: TOCTOU in atomic_write | ✅ Fixed | Permissions set before unlock |
| FIND-015: Lock held during cooldown iteration | ⚠️ Partially — scope reduced but lock still held for iteration (see FIND-013) |
| FIND-016: Non-atomic refresh cycle | ✅ Fixed | Now uses file locking for read-modify-write |
| FIND-017: Graceful shutdown | ⚠️ Partially — signal handler added but tasks not gracefully stopped (see FIND-025) |
| FIND-018: UsageMetrics never refreshes | ✅ Fixed | Now polls on interval |
| FIND-019: Hardcoded localhost | ✅ Fixed | Uses `appConfig.proxy_host` |
| FIND-020: Drag-and-drop index key | ✅ Fixed | Now uses stable key |
| FIND-021: providerEnabled not initialized | ✅ Fixed | Now fetched from backend |
| FIND-022: Countdown not reactive | ✅ Fixed | Now uses `setInterval` |
| FIND-023: CSP null | ✅ Fixed | CSP now configured |
| FIND-024: Filter dropdowns don't close | ⚠️ Partially — click-outside added but handler re-subscribes every render (see FIND-009) |
| FIND-025: sidecar/ not in .gitignore | ✅ Fixed | Added to .gitignore |
| FIND-026: Zero Tauri command tests | ⚠️ Partially — tests added but coverage still thin (see FIND-021) |
| FIND-027: Zero frontend tests | ❌ **NOT FIXED** — Still zero frontend tests (see FIND-022) |

---

## Critical Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-001 | Critical | `sidecar/src/proxy/server.rs:437-454` | **Streaming token counts always zero (Anthropic)** — `token_counts` are read synchronously immediately after creating the lazy stream body, before Axum has polled the stream. The translator has not yet extracted any tokens. Counts are always `(0, 0)`. All streaming Anthropic requests record zero tokens in both router state and metrics DB. |
| FIND-002 | Critical | `sidecar/src/proxy/server.rs:456-465` | **Streaming token counts always zero (OpenAI)** — OpenAI streaming responses are passed through as-is with hardcoded `(0, 0)`. No attempt is made to extract tokens from the SSE stream. Since streaming is the primary use case for AI coding assistants, all usage metrics, cost tracking, and quota enforcement are broken for streaming requests. |
| FIND-003 | Critical | `sidecar/src/proxy/server.rs:359-366`, `sidecar/src/proxy/router.rs:281-287`, `sidecar/src/metrics/scheduler.rs:82-130` | **`QuotaExhausted` status never set — recovery completely broken** — `process_response()` never returns `RequestError::QuotaExhausted`. It only returns `RateLimited`, `Network`, or `ServerError`. The `Err(RequestError::QuotaExhausted)` branch is dead code. `record_quota_exhausted()` is never called. `EntryStatus::QuotaExhausted` is never set at runtime. `run_quota_reset()` only processes `QuotaExhausted` entries, so it never resets anything. Over-quota entries are skipped in `select_entry()` but never automatically recovered without a process restart. |

## High-Severity Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-004 | High | `sidecar/src/proxy/server.rs:658-676`, `sidecar/src/metrics/scheduler.rs:267-312` | **In-memory groups not updated after `set_entry_enabled`** — `handle_internal_router_set_entry` saves updated groups to disk but `AppState.groups` (`Arc<Vec<Group>>`) is never updated. Subsequent calls to `handle_models` and `route_request` use stale in-memory groups. Router state is updated but group definitions (entries, priorities, failover config) are stale until restart. |
| FIND-005 | High | `sidecar/src/metrics/scheduler.rs:116-117` | **`run_quota_reset` doesn't reset `consecutive_errors`** — When recovering from quota exhaustion, sets `status = Active` and `daily_tokens_used = 0` but does not reset `consecutive_errors`. If the entry had accumulated errors before being quota-exhausted, those errors persist after recovery, potentially causing an immediate cooldown re-trigger. |
| FIND-006 | High | `sidecar/src/proxy/router.rs:307-320` | **`record_latency_timeout` doesn't reset `consecutive_errors`** — When a latency timeout triggers cooldown, `consecutive_errors` is not reset. A subsequent non-timeout error could immediately trigger the consecutive-errors cooldown. |
| FIND-007 | High | `sidecar/src/metrics/scheduler.rs:210` | **Scheduler probe has no timeout** — Probe request uses the global client's 120s timeout. A hung provider would block the probe for 2 minutes. Probes should have a much shorter timeout (e.g., 10s). |
| FIND-008 | High | `src-tauri/src/commands.rs:508-521` | **AppImage sidecar path detection is broken** — `std::env::var("APPIMAGE")` returns the path to the `.AppImage` file itself (e.g., `/home/user/CodeRouter.AppImage`). `.parent()` gives the directory containing the AppImage, not the FUSE mount point (e.g., `/tmp/.mount_CodeRXXXXX/`). The sidecar binary will never be found at that location when running from a bundled AppImage. |
| FIND-009 | High | `src/pages/UsageMetrics.tsx:121-130` | **`FilterDropdown` click-outside handler re-subscribes on every render** — `useEffect` has `onToggle` in its dependency array. `onToggle` is an inline arrow function created fresh on every render. The click-outside listener is removed and re-added on every render, which can cause the dropdown to close immediately after opening or behave erratically. |

## Medium-Severity Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-010 | Medium | `sidecar/src/proxy/router.rs:241-256` | **`record_success` doesn't check quota after incrementing** — Increments `daily_tokens_used` but doesn't check if quota has been exceeded. Entry remains `Active` even after exceeding quota. Concurrent requests can slip through before the next `select_entry` catches it. |
| FIND-011 | Medium | `sidecar/src/proxy/server.rs:151-216` | **`handle_models` doesn't check quota** — `GET /v1/models` filters by enabled status and active/cooldown state but does not check if entries have exceeded daily token quota. A group can appear in the model list even when all its entries are over quota, leading to 503 responses for clients. |
| FIND-012 | Medium | `sidecar/src/proxy/server.rs:152-153` | **`GET /v1/models` uses stale in-memory groups** — Uses `state.groups` (loaded at startup) but `load_providers()` reads from disk. If groups are modified via Tauri commands, the model list shows stale data. |
| FIND-013 | Medium | `sidecar/src/metrics/scheduler.rs:146-187` | **`run_cooldown_check` holds router state lock while iterating all groups** — The lock is held for the entire iteration. During this time, the proxy server cannot access router state for request routing. With many groups/entries, this causes latency spikes every 30 seconds. |
| FIND-014 | Medium | `sidecar/src/proxy/server.rs:422-425` | **`process_response` reads error body for streaming responses** — For streaming responses that return a non-success status, `resp.text().await` is called. If the upstream returned a streaming error response, this could hang indefinitely. |
| FIND-015 | Medium | `sidecar/src/proxy/server.rs:253` | **`route_request` reloads providers from disk every request** — `load_providers()` is called on every request. This is correct for freshness but inconsistent with in-memory groups, causing the two data sources to diverge. |
| FIND-016 | Medium | `sidecar/src/proxy/server.rs:368-379` | **`record_consecutive_error` only tracks when trigger enabled** — The call is inside `if group.failover_config.on_consecutive_errors`, so error counting only happens when the trigger is enabled. If the trigger is later enabled, the counter starts from zero. |
| FIND-017 | Medium | `sidecar/src/models/refresher.rs:99` | **Model refresher has no timeout on detail requests** — Individual model detail fetches use the global client's 120s timeout. For providers with hundreds of models, this could take hours. |
| FIND-018 | Medium | `src/components/ui/` (8 components) | **shadcn/ui components installed but barely used** — `Button`, `Card`, `Badge`, `Dialog`, `Tabs`, `Table`, `Progress`, `Select` are generated but only `Card`/`Button` used in Settings. All other pages use custom implementations. `Dialog` is installed but modals use custom div-based overlays. `Select` is installed but all dropdowns use native `<select>`. |
| FIND-019 | Medium | `src/types/index.ts:18-28` | **TypeScript `Provider` type missing `model_overrides` field** — Rust `Provider` struct includes `model_overrides: Option<Vec<ProviderModel>>` but TS interface omits it. If a provider with model overrides is edited and saved, the `model_overrides` data would be lost. |
| FIND-020 | Medium | `src/types/index.ts:57-62` | **TypeScript `AppConfig` type missing `opencode_config_path` field** — Rust `AppConfig` includes `opencode_config_path: Option<String>` but TS interface omits it. |
| FIND-021 | Medium | `src-tauri/src/commands.rs` | **Tauri command test coverage still thin** — Tests were added but coverage of the IPC command handlers remains thin. Edge cases in error handling, concurrent operations, and state transitions are not tested. |
| FIND-022 | Medium | `src/` (all frontend files) | **Zero frontend tests** — No `.test.ts` or `.test.tsx` files exist. The entire React frontend is untested. `package.json` test script is a no-op echo. |
| FIND-023 | Medium | `sidecar/src/config/store.rs:123` | **`OpenOptions` with `.create(true)` but no `.truncate(true)`** — If the file already exists with content, subsequent `read_to_string` + `serde_json::from_str` could fail on partial overwrites. `set_len(0)` is called later but there's a read between open and truncate that could be stale. |
| FIND-024 | Medium | `src-tauri/src/commands.rs:99, 147` | **`save_provider`/`save_group` use `unwrap_or_default()` on load** — If the providers/groups file is corrupted, it silently starts fresh without warning. Data loss with no user notification. |
| FIND-025 | Medium | `sidecar/src/proxy/server.rs:83, 87-95`, `sidecar/src/metrics/scheduler.rs:64-79` | **Background tasks have no graceful shutdown** — Scheduler and refresher task handles are dropped immediately. They run forever with no shutdown mechanism. On SIGTERM/SIGINT, tasks are abandoned mid-operation. Metrics recorder channel is not drained on shutdown, losing pending events. |
| FIND-026 | Medium | `build.sh:9, 15, 20, 32` | **Build script issues** — Hardcoded to `x86_64-unknown-linux-gnu` target. `cargo build` doesn't specify `--target`, so if default target differs, the copied binary has the wrong name. AppImage path assumes specific bundle layout. No check for `appimagetool`/`linuxdeploy` dependency. |
| FIND-027 | Medium | `Cargo.toml` (root) | **No `[workspace.dependencies]`** — Both `src-tauri` and `sidecar` independently specify `tokio`, `serde`, `serde_json`, `reqwest`, `rusqlite`, `chrono`. Should be unified via workspace dependencies to avoid version drift. |
| FIND-028 | Medium | `src/pages/Dashboard.tsx:269-285` | **ProviderHealthCards summaries never refresh after initial load** — `useEffect` depends only on `[providers]`. Once loaded, summaries are never refreshed — not on a timer, not on navigation return. "Today" data becomes stale after midnight UTC. |
| FIND-029 | Medium | `src/pages/ModelGroups.tsx:259, 376` | **Duplicate group status polling** — `GroupCard` calls `useGroupStatusPoll(group.id)`. It then passes `group.entries` to `LiveStatusPanel`, which calls `useGroupStatusPoll(groupId)` again. Two independent 5-second polling intervals for the same group, doubling IPC traffic and proxy requests per group. |
| FIND-030 | Medium | `src/pages/ModelGroups.tsx:590-599` | **`handleDragOver` causes excessive re-renders** — `onDragOver` fires dozens of times per second. Each fire calls `setEntries()` and `setDragIdx()`, triggering full component re-renders. With many entries, noticeable jank occurs. |
| FIND-031 | Medium | `src/store/index.ts:30-43` | **`loadInitialData` fails atomically** — Uses `Promise.all`. If any one of the three IPC calls fails, the entire promise rejects and none of the successfully-loaded data is stored. Should use `Promise.allSettled` or individual `try/catch`. |
| FIND-032 | Medium | `src/pages/Providers.tsx:133, 146, 159` | **Unnecessary dynamic imports in Providers.tsx** — `getProviders` is already statically imported but delete, save, and toggle handlers use `await import('../lib/ipc')).getProviders()`. Inconsistent and could cause subtle timing issues. |
| FIND-033 | Medium | `src/pages/ModelGroups.tsx:427, 706` | **Drag-and-drop keys can collide for same provider+model entries** — Both `LiveStatusPanel` and `GroupForm` use `key={\`${entry.providerId}-${entry.modelId}-${idx}\`}`. If a group has two entries with the same provider ID and model ID (valid use case), keys collide, causing React reconciliation bugs during reordering. |

## Low-Severity Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-034 | Low | `sidecar/src/proxy/server.rs:28` | Unused import: `get_latency_percentiles` |
| FIND-035 | Low | `sidecar/src/proxy/server.rs:568` | Dead code: `AppError::UpstreamError` variant never constructed |
| FIND-036 | Low | `sidecar/src/metrics/scheduler.rs:242` | Dead code: `ProbeResult::Error(String)` field payload always discarded |
| FIND-037 | Low | `sidecar/src/models/refresher.rs:282-284` | Dead code: `is_leap_year` function defined but never called |
| FIND-038 | Low | `sidecar/src/proxy/translator.rs:458-466` | `uuid_short` is not a real UUID — uses timestamp + random bits, could produce collisions |
| FIND-039 | Low | `sidecar/src/models/refresher.rs:278-394` | Hand-rolled ISO 8601 parsing instead of using `chrono` |
| FIND-040 | Low | `sidecar/src/proxy/server.rs:179` | `/v1/models` returns `created: 0` for all models |
| FIND-041 | Low | `sidecar/src/proxy/server.rs:90-91` | `refresh_interval_hours` could be 0, causing `tokio::time::sleep(0)` tight loop |
| FIND-042 | Low | `sidecar/src/proxy/ssrf.rs:38-53` | SSRF validation doesn't prevent DNS rebinding — domain names checked by string matching only |
| FIND-043 | Low | `src-tauri/src/commands.rs:128-138` | `delete_provider` doesn't clean up groups referencing the deleted provider |
| FIND-044 | Low | `src-tauri/src/commands.rs:96-116` | `save_provider` doesn't validate `protocol` field — any string accepted, invalid values silently default to OpenAI behavior |
| FIND-045 | Low | `src-tauri/src/main.rs:110` | `poll_health` uses `unwrap()` on Mutex — if mutex is poisoned, the entire health poll task panics |
| FIND-046 | Low | `src/pages/OpenCodeSetup.tsx:110` | `mappingForPreview` silently drops `compaction`, `title`, `summary` from `hasAnyMapping` check |
| FIND-047 | Low | `src/pages/ModelGroups.tsx:41-47` | `formatTimestamp` uses `toLocaleTimeString()` — loses date context. Inconsistent with Providers.tsx which uses `toLocaleString()`. |
| FIND-048 | Low | `src/pages/Providers.tsx:67`, `ModelGroups.tsx:118`, `OpenCodeSetup.tsx:70`, `Settings.tsx:31` | Toast ID collisions possible with `Date.now()` — rapid sequences could share an ID |
| FIND-049 | Low | `src/pages/Dashboard.tsx:118-131`, `src/pages/UsageMetrics.tsx:646-658` | `StatusBadge` component duplicated in two files |
| FIND-050 | Low | `src/pages/UsageMetrics.tsx:345` | CSV export doesn't handle newlines in field values |
| FIND-051 | Low | `src-tauri/src/commands.rs:589` | `test_group` helper is dead code in test module |
| FIND-052 | Low | `src-tauri/src/commands.rs:169` | Redundant closure `|c| Ok(c)` should be `Ok` |
| FIND-053 | Low | `src-tauri/src/main.rs:50` | Complex return type should be factored into a `type` alias |
| FIND-054 | Low | `Makefile` | No `clean` target |
| FIND-055 | Low | `index.html:5` | References `/vite.svg` default favicon instead of CodeRouter branding |
| FIND-056 | Low | `package.json:11-14` | Test, lint, typecheck, format scripts are all no-op echo commands |

---

## Design Gaps vs plan.md

| ID | Area | Description |
|---|---|---|
| DG-001 | Backend | **`daily_request_quota` not implemented** — Plan mentions "daily request quota" as optional provider field. Only `daily_token_quota` exists. |
| DG-002 | Backend | **Latency timeout cooldown not configurable** — Plan: "Cooldown period (configurable, default 5 minutes)." Hardcoded to 5 minutes. |
| DG-003 | Backend | **Consecutive errors cooldown not configurable** — Plan: "Fixed cooldown period (configurable, default 10 minutes)." Hardcoded to 10 minutes. |
| DG-004 | Backend | **`/v1/completions` not "forwarded as-is"** — Plan: "Legacy completions endpoint (forwarded as-is if provider supports it)." Implementation translates to Anthropic messages format, losing system prompts and conversation history. |
| DG-005 | Backend | **OpenCode config writer uses static config status, not runtime state** — `inject_provider` and `preview_opencode_config` filter entries by `e.status == "active"` (static config string), not runtime `EntryStatus`. |
| DG-006 | Backend | **`opencode.json` cached config not implemented** — Plan lists `~/.config/coderouter/opencode.json`. Not read or written. |
| DG-007 | Backend | **System agents `compaction`, `title`, `summary` missing** — Plan lists 7 agents. `AgentMapping` only has `build`, `plan`, `general`, `explore`, `small_model`. |
| DG-008 | Frontend | **No first-run onboarding flow** — Plan Phase 6 mentions "First-run onboarding flow." Onboarding component exists but dismissal state is not persisted — shows again on every launch. |
| DG-009 | Frontend | **No streaming metrics display** — Plan specifies "Local HTTP (on a second internal port, e.g. 4142): for metrics/log streaming to the UI." Frontend implements no WebSocket, SSE, or EventSource connections. |

---

## Summary by Severity

| Severity | Count |
|---|---|
| Critical | 3 |
| High | 7 |
| Medium | 24 |
| Low | 23 |
| **Total** | **57** |

## Summary by Category

| Category | Count |
|---|---|
| Backend Bugs | 25 |
| Frontend Bugs | 15 |
| Design Gaps | 9 |
| Build/Infrastructure | 5 |
| Test Coverage | 3 |

---

## Recommended Priority Order for Fixes

1. **FIND-001/002** — Fix streaming token extraction (Anthropic: read counts after stream consumption; OpenAI: parse SSE stream for token data)
2. **FIND-003** — Wire up `QuotaExhausted` status in `process_response` and ensure `run_quota_reset` actually resets entries
3. **FIND-004** — Update `AppState.groups` after `set_entry_enabled` saves to disk
4. **FIND-008** — Fix AppImage sidecar path detection (use `/tmp/.mount_*/` pattern or `APPDIR` env var)
5. **FIND-005/006** — Reset `consecutive_errors` on quota recovery and latency timeout
6. **FIND-007** — Add short timeout (10s) to scheduler probe requests
7. **FIND-009** — Memoize `onToggle` in `FilterDropdown` to prevent click-outside re-subscription
8. **FIND-018** — Either adopt shadcn/ui components or remove the unused generated files
9. **FIND-022** — Add at least basic frontend tests (Vitest + React Testing Library)
10. **FIND-029** — Deduplicate group status polling (single poll per group, shared between Card and Panel)

---

## Notes

- 102 Rust tests pass (up from 86 in Audit 002, 76 in Audit 001). TypeScript check is clean.
- Zero frontend tests. Tauri command test coverage remains thin.
- The streaming token extraction issue (FIND-001/002) is the single most impactful bug — it breaks the core value proposition of usage tracking and cost management for the primary use case (streaming requests).
- The `QuotaExhausted` system being non-functional (FIND-003) means providers that hit their daily quota will be silently skipped in routing but never recovered, requiring a manual restart.
- This is Audit 003 — subsequent audits should track remediation of these findings.
