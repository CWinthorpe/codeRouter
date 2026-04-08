# CodeRouter — Audit Report 002

**Date:** 2026-04-08  
**Scope:** Full project re-audit — bugs, design gaps, security, and completeness (post-fix review)  
**Reference:** `docs/plan.md` (design spec), `docs/audit/audit-001-initial-audit.md` (prior audit)  
**Build Status:** 86/86 Rust tests passing (+10 from prior audit), TypeScript check clean

---

## Executive Summary

This is the second audit of CodeRouter, conducted after the findings from Audit 001 were addressed. The project has improved — test count increased from 76 to 86, and many critical issues from the first audit have been resolved. However, **33 new or remaining findings** were identified across the Rust sidecar, React frontend, and project infrastructure.

**The most impactful remaining issues are:**

- **Router state is never initialized in the Tauri process** — `get_router_status` and `set_entry_enabled` IPC commands always fail with "Router state not initialized" because the `GLOBAL_ROUTER_STATE` `OnceLock` is only set in the sidecar binary, not the Tauri process that handles IPC.
- **Streaming requests always record 0 tokens** — both Anthropic and OpenAI streaming paths return `(response, 0, 0)` for token counts, breaking usage metrics, cost tracking, and quota enforcement for all streaming requests (the primary use case for AI coding assistants).
- **Model refresher runs once at startup, never on a schedule** — the `refresh_interval_hours` config exists but no recurring timer is set up. Models are fetched once and never refreshed.
- **Anthropic streaming SSE is double-encoded** — the translator wraps already-formatted SSE data lines in another `Event::default().data()` call, producing `data: data: {...}` which breaks client parsing.
- **shadcn/ui is incomplete and unused** — components are generated but Tailwind theme variables are not mapped, and all pages use custom implementations instead of shadcn components.

---

## Previous Audit Fix Verification

| Audit 001 Issue | Status | Notes |
|---|---|---|
| BUG-001: Double scheduler spawn | ✅ Fixed | Only one `spawn_scheduler` call remains |
| BUG-002: Metrics recording absent | ✅ Fixed (non-streaming) | `MetricsRecorder` is now wired in for non-streaming requests |
| BUG-003: Token quota dead code | ✅ Fixed (non-streaming) | Token counts are now extracted and passed for non-streaming |
| BUG-004: Streaming SSE corruption | ⚠️ Partially fixed | Streaming works but SSE is double-encoded (see FIND-003) |
| BUG-005: API key overwritten on toggle | ✅ Fixed | Rust now checks `!api_key.is_empty()` before storing |
| BUG-006: Error counter never resets | ✅ Fixed | `record_success` resets `consecutive_errors` to 0 |
| BUG-007: ProbeGuard key leak | ✅ Fixed | `Drop` impl now properly removes key on failure |
| BUG-008: Quota reset loop | ✅ Fixed | Uses `daily_reset_at` to gate resets |
| BUG-009: JSON parse silence | ✅ Fixed | Proper error handling for invalid JSON |
| BUG-010: No latency tracking | ✅ Fixed | Latency is now measured and recorded |
| BUG-011: Missing React Fragment | ✅ Fixed | `Dashboard.tsx` uses proper fragments |
| BUG-012: activeEntry logic | ✅ Fixed | Logic now correctly finds highest-priority active entry |
| BUG-013: Inline components | ✅ Fixed | Components extracted to module scope |
| GAP-001: proxy::upstream missing | ✅ Fixed | New `upstream.rs` module created |
| GAP-002: Model refresher not scheduled | ❌ **NOT FIXED** | Still runs once at startup only (see FIND-002) |
| GAP-003: /v1/models missing metadata | ✅ Fixed | Metadata now included in response |
| GAP-004: System agents missing | ⚠️ Partially | `small_model` present; `compaction`/`title`/`summary` still absent |
| GAP-005: opencode.json cached config | ❌ **NOT FIXED** | Still not implemented |
| GAP-006: No latency percentiles | ⚠️ Partially | Backend endpoint exists but unused in frontend |
| GAP-007: Daily totals not persisted | ⚠️ Partially | `init_daily_totals_from_db` added but uses UTC midnight, not provider reset hour |
| GAP-008: /v1/completions bare passthrough | ⚠️ Partially | Now routes through upstream module but Anthropic translation is lossy |
| GAP-009: /health incomplete | ✅ Fixed | Now returns active providers and failover states |
| GAP-010: shadcn/ui not configured | ⚠️ Partially | Components generated but theme vars not mapped, components unused |
| GAP-011: Config path not functional | ✅ Fixed | `setOpencodeConfigPath` IPC wired and used |
| GAP-012: Missing remove button | ✅ Fixed | Dedicated remove button added |
| SCH-001–003: Schema renames | ✅ Fixed | All serde renames match plan.md schemas |
| SEC-001: SSRF risk | ✅ Fixed | `ssrf.rs` validation module added |
| SEC-003: OpenCode writer no atomic write | ✅ Fixed | Now uses atomic write pattern |
| FAI-001: QuotaExhausted never probed | ⚠️ Partially | Scheduler now probes QuotaExhausted entries with stale cooldown_until (see FIND-008) |
| FAI-002: set_entry_enabled only persists disable | ✅ Fixed | Now persists both enable and disable |
| TAU-001: restart_proxy tray state | ✅ Fixed | Tray/menu state now updated |

---

## Critical Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-001 | Critical | `src-tauri/src/main.rs`, `src-tauri/src/commands.rs:340-355`, `sidecar/src/proxy/router.rs:9-17` | **Router state never initialized in Tauri process** — `GLOBAL_ROUTER_STATE` `OnceLock` is only set by `init_and_set_global_router_state()` called from the sidecar binary's `start_server()`. The Tauri process spawns the sidecar as a child but never initializes router state in its own address space. `get_router_status()` always returns `"Router state not initialized"` and `set_entry_enabled()` always fails. The Live Status Panel in the UI and manual entry enable/disable are completely non-functional. |
| FIND-002 | Critical | `sidecar/src/proxy/server.rs:85-87` | **Model refresher runs once at startup, never on a schedule** — `refresh_all_providers` is spawned once at startup. The plan specifies "daily refresh schedule" and `refresh_interval_hours` config exists (default 24), but no recurring timer is set up. Models are fetched once and never refreshed unless manually triggered. |
| FIND-003 | Critical | `sidecar/src/proxy/server.rs:411-427`, `sidecar/src/proxy/translator.rs:389` | **Streaming requests always record 0 tokens + double-encoded SSE** — Both Anthropic and OpenAI streaming paths return `(response, 0, 0)` for token counts. All streaming requests (the primary use case for AI coding assistants) record zero input/output tokens, breaking usage metrics, cost tracking, and daily quota enforcement. Additionally, `translate_anthropic_stream` wraps already-formatted SSE data lines (`data: {...}\n\n`) in another `Event::default().data()`, producing double-encoded output (`data: data: {...}`) that breaks client parsing. |

## High-Severity Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-004 | High | `sidecar/src/proxy/router.rs:237-248` | **`record_success` does not transition Cooldown → Active** — When a cooldown-expired entry successfully handles a request, its status remains `EntryStatus::Cooldown` forever. The `cooldown_until` field is never cleared and the status is never set to `Active`. This means the entry stays invisible in `/v1/models` metadata and future cooldown checks keep trying to probe an already-working entry. |
| FIND-005 | High | `sidecar/src/proxy/router.rs:258-265` | **429 exponential backoff starts at 120s, not 60s** — `base_backoff_seconds` is 60, but the first 429 computes `60 * 2 = 120`, skipping the initial 60s cooldown. Plan says "starting at 60s, doubling up to 1 hour max". Fix: use `current_backoff` directly on first occurrence, only double on subsequent 429s. |
| FIND-006 | High | `sidecar/src/metrics/queries.rs:237-261`, `sidecar/src/proxy/router.rs:99-125` | **Daily token totals use UTC midnight, not provider-specific reset hour** — `get_today_token_totals` uses `now.date_naive().and_hms_opt(0, 0, 0)` (always UTC midnight). Providers can configure `quota_reset_utc_hour` to any hour (e.g., 6 for 06:00 UTC). Daily quota tracking is off by up to 24 hours of misaligned data. |
| FIND-007 | High | `sidecar/src/proxy/upstream.rs:74-94` | **Timeout only covers `send()`, not response body read** — `tokio::time::timeout` wraps only `req.send()`, which completes when response headers are received. For streaming responses, the body can take arbitrarily long to read without triggering the timeout. A provider could start responding then hang indefinitely without failover. |
| FIND-008 | High | `sidecar/src/metrics/scheduler.rs:157-168` | **Quota-exhausted entries with stale `cooldown_until` get incorrectly probed** — If an entry was previously in a 429 cooldown (which sets `cooldown_until`) and then gets quota-exhausted, the old `cooldown_until` persists. The scheduler probes the quota-exhausted entry when the stale cooldown expires, which is wrong — quota exhaustion should only recover via the daily reset timer. |
| FIND-009 | High | `src-tauri/src/commands.rs:523-539` | **`restart_proxy` leaves stale router state in Tauri process** — After killing and respawning the sidecar, the Tauri process's `GLOBAL_ROUTER_STATE` still holds the old (now-stale) router state. Even if FIND-001 were fixed, a restart would leave the Tauri process with router state that doesn't match the new sidecar process. No IPC exists for router state synchronization between processes. |
| FIND-010 | High | `tailwind.config.js:7-8` | **shadcn/ui CSS variables not mapped in Tailwind theme** — `theme.extend` is empty. shadcn/ui components reference CSS variables (`bg-background`, `text-foreground`, `border-border`, `bg-card`, etc.) that are defined in `src/index.css` but not mapped in Tailwind's theme config. All shadcn components (`Button`, `Card`, `Badge`, `Dialog`, `Select`, `Table`, `Tabs`, `Progress`) will render with broken colors if used. Currently all pages use custom implementations, so shadcn components are dead code. |

## Medium-Severity Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-011 | Medium | `sidecar/src/proxy/server.rs:146-171` | **`handle_models` picks highest-priority entry without checking runtime status for metadata** — Filters only `e.enabled`, not runtime `EntryStatus`. If priority 1 is in cooldown, it's still selected as "highest active" and its metadata is used. The `is_active` check then returns `None` for context_window/max_output_tokens, so the model appears with no metadata instead of falling through to the next actually-active entry. |
| FIND-012 | Medium | `sidecar/src/proxy/router.rs:42-44` | **`EntryState::new` panics on invalid `quota_reset_utc_hour`** — `and_hms_opt(reset_hour, 0, 0).unwrap()` panics if `reset_hour >= 24`. `Provider.quota_reset_utc_hour` is `u32` with no validation. Same issue in `scheduler.rs:109-111`. |
| FIND-013 | Medium | `sidecar/src/proxy/server.rs:393-399` | **429 detection misses non-SSE error responses in streaming path** — If upstream returns 429 with a non-SSE content type (e.g., HTML error page), it falls through to JSON parsing which fails, returning `RequestError::Network` instead of `RequestError::RateLimited`. Failover logic for 429 is bypassed. |
| FIND-014 | Medium | `sidecar/src/config/store.rs:47-56` | **`atomic_write` sets permissions after unlock — TOCTOU window** — File is unlocked at line 51, then permissions set at line 55. Between unlock and `set_permissions`, another process could read or modify the file. Should set permissions before unlocking. |
| FIND-015 | Medium | `sidecar/src/metrics/scheduler.rs:146-187` | **`run_cooldown_check` holds router state lock while iterating all groups** — The `guard` (router state lock) is held for the entire iteration. During this time, the proxy server cannot access router state for request routing. With many groups/entries, this causes noticeable latency spikes every 30 seconds. |
| FIND-016 | Medium | `sidecar/src/models/refresher.rs:460-470` | **`refresh_single_provider` has non-atomic load-modify-save cycle** — Between `load_providers()` and `save_providers()`, another process could modify the providers file. `atomic_write` with file locking protects individual writes but not the read-modify-write cycle. |
| FIND-017 | Medium | `sidecar/src/proxy/server.rs:103` | **No graceful shutdown handling** — `axum::serve(listener, app).await` has no signal handler (SIGTERM, SIGINT). In-flight requests are abruptly terminated when the process is killed. |
| FIND-018 | Medium | `src/pages/UsageMetrics.tsx:186-199` | **Request data fetched only once on mount, never refreshed** — `useEffect` has `[]` dependency. Unlike Dashboard and ModelGroups which poll every 5 seconds, UsageMetrics never refreshes. Users staying on this page see stale data. |
| FIND-019 | Medium | `src/hooks/useProxyStatusPoll.ts:11`, `src/pages/Dashboard.tsx:19` | **Health poll uses hardcoded `localhost` instead of `appConfig.proxy_host`** — If user changes `proxy_host` to `0.0.0.0` or any other address, health check URL may not match. Port is correctly read from `appConfig.proxy_port` but host is always `localhost`. |
| FIND-020 | Medium | `src/pages/ModelGroups.tsx:696-704` | **Drag-and-drop uses array index as React `key`** — `key={idx}` breaks React reconciliation when items are reordered. DOM state (quota override inputs, toggle states) gets associated with wrong entries after drag-and-drop. Should use stable identifier like `${entry.providerId}-${entry.modelId}`. |
| FIND-021 | Medium | `src/pages/OpenCodeSetup.tsx:52` | **`providerEnabled` state never initialized from backend** — Toggle starts as `false` and is never fetched from backend. UI always shows "Not configured" on first load even if previously configured. |
| FIND-022 | Medium | `src/pages/ModelGroups.tsx:434` | **Cooldown countdown is not reactive** — `cooldownCountdown` computes from `Date.now()` at render time but has no `setInterval` to update. Countdown only updates every 5 seconds (on poll cycle), not in real-time. |
| FIND-023 | Medium | `src-tauri/tauri.conf.json:22` | **CSP is `null`** — Content Security Policy is entirely disabled. At minimum, a restrictive CSP should be set for production builds. |
| FIND-024 | Medium | `src/pages/UsageMetrics.tsx:114-149` | **Filter dropdowns don't close on outside click** — `FilterDropdown` shows/hides via local `show` state with no click-outside handler. Dropdowns stay open if user clicks elsewhere. |
| FIND-025 | Medium | `.gitignore` | **`src-tauri/sidecar/` not in `.gitignore`** — The compiled sidecar binary (~13MB) is staged in `src-tauri/sidecar/` but not ignored. Risk of accidentally committing a large binary. |
| FIND-026 | Medium | `src-tauri/src/main.rs`, `src-tauri/src/commands.rs` | **Zero test coverage for Tauri commands layer** — All 86 tests are in the sidecar library. The IPC command handlers (`commands.rs`) have no tests. |
| FIND-027 | Medium | Frontend (all pages) | **Zero TypeScript/frontend tests** — No `.test.ts` or `.test.tsx` files exist. The entire React frontend is untested. |

## Low-Severity Findings

| ID | Severity | File(s) | Description |
|---|---|---|---|
| FIND-028 | Low | `sidecar/src/proxy/server.rs:28` | Unused import: `get_latency_percentiles` |
| FIND-029 | Low | `sidecar/src/proxy/server.rs:568` | Dead code: `AppError::UpstreamError` variant never constructed |
| FIND-030 | Low | `sidecar/src/metrics/scheduler.rs:242` | Dead code: `ProbeResult::Error(String)` field payload always discarded via `Error(_)` pattern |
| FIND-031 | Low | `sidecar/src/proxy/translator.rs:458-466` | `uuid_short` is not a real UUID — uses timestamp + random bits, could produce collisions |
| FIND-032 | Low | `sidecar/src/models/refresher.rs:278-394` | Hand-rolled ISO 8601 parsing instead of using `chrono`. Doesn't handle timezones, leap seconds, or fractional seconds. |
| FIND-033 | Low | `sidecar/src/proxy/server.rs:179` | `/v1/models` returns `created: 0` for all models — valid but not useful |
| FIND-034 | Low | `src/pages/Providers.tsx:137` | Stale closure on `providers` in `handleDelete` — rapid double-click could use stale list |
| FIND-035 | Low | `src/pages/Providers.tsx:445` | Daily token quota validation allows negative numbers — input has `min="0"` but can be bypassed |
| FIND-036 | Low | `src/pages/Settings.tsx:58` | Port validation allows non-integer values (e.g., `4141.5` passes range check) |
| FIND-037 | Low | `src/pages/ModelGroups.tsx:504` | `showFailover` state persists across group edits — opening failover for one group leaves it open for the next |
| FIND-038 | Low | `src/pages/Settings.tsx:152-165` | Settings page has inline toast implementation instead of using shared `Toast` component |
| FIND-039 | Low | `src/pages/OpenCodeSetup.tsx:121-134` | Double preview fetch on mount — two `useEffect` hooks both call `fetchPreview()`, one immediate and one debounced |
| FIND-040 | Low | `src/lib/ipc.ts:130-132, 154-159` | `getLatencyPercentiles`, `getUsageByDay`, `getUsageByGroup` defined in IPC but never used in any component |
| FIND-041 | Low | `src/pages/Dashboard.tsx:367` | `showExpand` logic doesn't account for `failover` status rows being expandable |
| FIND-042 | Low | `src/pages/UsageMetrics.tsx:293` | Timezone mismatch in chart data — `getDaysInRange` uses local time while day extraction from `r.ts` uses UTC. Requests could be misattributed or dropped. |
| FIND-043 | Low | `package.json` | No `test`, `lint`, `typecheck`, or `format` scripts defined |

---

## Design Gaps vs plan.md

| ID | Area | Description |
|---|---|---|
| DG-001 | Phase 6 | **No first-run onboarding flow** — Plan mentions "First-run onboarding flow" in Phase 6. No onboarding component exists. |
| DG-002 | Backend | **`opencode.json` cached config not implemented** — Plan lists `~/.config/coderouter/opencode.json` as "Cached OpenCode integration settings". Not read or written. |
| DG-003 | Backend | **System agents `compaction`, `title`, `summary` missing** — Plan lists 7 agents. `AgentMapping` only has `build`, `plan`, `general`, `explore`, `small_model`. |
| DG-004 | Frontend | **shadcn/ui components generated but unused** — All shadcn components (`Button`, `Card`, `Dialog`, `Select`, `Table`, `Tabs`, `Progress`, `Badge`) are set up but never imported. All pages use custom implementations with raw Tailwind classes. |
| DG-005 | Backend | **No request caching** — Listed in plan's "Open Questions / Future Considerations". Not implemented. |
| DG-006 | Backend | **No load balancing mode** — Listed in plan's "Open Questions". Not implemented. |
| DG-007 | Backend | **No rate limiting on local endpoint** — Listed in plan's "Open Questions". Not implemented. |

---

## Summary by Severity

| Severity | Count |
|---|---|
| Critical | 3 |
| High | 7 |
| Medium | 17 |
| Low | 16 |
| **Total** | **43** |

## Summary by Category

| Category | Count |
|---|---|
| Backend Bugs | 17 |
| Frontend Bugs | 12 |
| Design Gaps | 7 |
| Security | 1 |
| Test Coverage | 3 |
| Build/Infrastructure | 3 |

---

## Recommended Priority Order for Fixes

1. **FIND-001** — Router state initialization in Tauri process (Live Status Panel and entry management completely broken)
2. **FIND-003** — Streaming token extraction + SSE double-encoding (all streaming metrics broken)
3. **FIND-002** — Schedule model refresher with recurring timer (models never refresh)
4. **FIND-004** — Transition Cooldown → Active on successful request (entries stuck in cooldown)
5. **FIND-005** — Fix 429 backoff to start at 60s, not 120s
6. **FIND-006** — Use provider-specific `quota_reset_utc_hour` in daily token query
7. **FIND-007** — Extend timeout to cover response body read for streaming
8. **FIND-009** — Reinitialize router state in Tauri process after restart
9. **FIND-010** — Map shadcn/ui CSS variables in Tailwind theme or remove unused shadcn components
10. **FIND-025** — Add `src-tauri/sidecar/` to `.gitignore`

---

## Notes

- 86 Rust tests pass (up from 76 in Audit 001). TypeScript check is clean.
- No frontend tests exist. No Tauri command tests exist. No doc tests exist.
- The architectural split between the Tauri process and sidecar process (FIND-001, FIND-009) is a fundamental design issue that may require rethinking how router state is shared — either via IPC to the sidecar's internal HTTP port, or by consolidating state management.
- This is Audit 002 — subsequent audits should track remediation of these findings.
