# CodeRouter — Audit Report 007

**Date:** 2026-04-08  
**Scope:** Fix verification audit — checking Audit 006 fixes were applied correctly and no new bugs introduced  
**Reference:** `docs/audit/audit-006-sixth-audit.md` (prior audit)  
**Build Status:** 116 Rust tests passing (13 types + 103 sidecar), TypeScript check clean

---

## Executive Summary

This audit verifies the fixes from Audit 006. **116 Rust tests pass** (up from 106), TypeScript is clean.

**Of the 23 findings from Audit 006:**
- **14 fixed correctly** ✅
- **2 fixed but have issues** ⚠️
- **2 not fixed** ❌
- **5 removed** (code quality items from prior cleanup)

**8 new bugs introduced by the fixes.**

---

## Fix Verification

### Critical Findings (Audit 006)

| Finding | Status | Notes |
|---|---|---|
| FIND-001: Counter divergence across entries | ✅ Fixed | `record_success` now iterates all entries with matching `provider_id` prefix and increments counters for all. Tests confirm cross-entry sync. |
| FIND-002: TimeoutStream cuts off streaming | ⚠️ Fixed with caveat | Now implements inter-chunk gap timeout (not total body). A stream that pauses for >30s between bursts would still be killed. The audit recommended 120s+. |
| FIND-003: Tauri can't access router state | ✅ Fixed | Now fetches from `/internal/router/status` HTTP endpoint. Returns empty map on failure (graceful degradation). |
| FIND-004: Failed streams recorded as success | ✅ Fixed | Callback receives `true`/`false` for success/error. Records `"error"` status with `error_type: "stream_error"` on failure. |

### High-Severity Findings (Audit 006)

| Finding | Status | Notes |
|---|---|---|
| FIND-005: init_db() called twice | ✅ Fixed | Removed from `ensure_first_run()`. Only `start_server()` calls it now. |
| FIND-006: get_usage_by_day GROUP BY wrong | ✅ Fixed | Uses `DATE(ts - reset_hour * 3600, 'unixepoch')` for both SELECT and GROUP BY. |
| FIND-007: get_usage_by_group ignores reset hour | ⚠️ Fixed with issue | Query now accepts `reset_hour` parameter, but the Tauri command at `commands.rs:418` **hardcodes `reset_hour = 0`**. Frontend always gets UTC-midnight-aligned data regardless of provider settings. |
| FIND-008: Body read has no timeout | ✅ Fixed | Both streaming and non-streaming paths wrap `resp.bytes()` in `tokio::time::timeout(120s)`. |
| FIND-009: Stale router entries accumulate | ✅ Fixed | After merge, `guard.entries.retain()` removes keys not in new config. |
| FIND-010: daily_request_quota not exposed | ✅ Fixed | Added to `ProviderResponse` struct and `From<&Provider>` impl with serde rename. |
| FIND-011: localhost vs 127.0.0.1 | ✅ Fixed | `useProxyStatusPoll.ts` now defaults to `'127.0.0.1'`. |
| FIND-012: ProviderModal state not resetting | ✅ Fixed | Uses `key={editingProvider ? editingProvider.id : 'new'}` to force remount. |
| FIND-013: Sidecar binary naming | ❌ Not fixed | `build.sh` copies with target-triple suffix, `tauri.conf.json` references without suffix. Tauri v2 doesn't auto-append. |

### Medium-Severity Findings (Audit 006)

| Finding | Status | Notes |
|---|---|---|
| FIND-014: Settings reset doesn't clear store | ✅ Fixed | `resetAll()` now resets all 7 fields: providers, groups, appConfig, proxyStatus, healthData, loadError, recentStreamRequests. |
| FIND-015: Drag-and-drop state not atomic | ✅ Fixed | React 18 automatic batching ensures `setEntries` and `setEntryKeys` are batched into single re-render. |
| FIND-016: SearchableSelect loses value | ✅ Fixed | Now uses `value={open ? search : selectedLabel}` and `onFocus` sets `setSearch(selectedLabel)` instead of clearing. |
| FIND-017: Timezone mismatch in date inputs | ❌ Not fixed | Still uses local timezone throughout `UsageMetrics.tsx`. `formatDate`, `startOfDay`, and `customStart + 'T00:00:00'` all use local time. |
| FIND-018: modelOverrides not returned | ✅ Fixed | Added to `ProviderResponse` struct and `From<&Provider>` impl. |
| FIND-019: quota_exhausted not counted | ✅ Fixed | `getEntryStatusCounts` now counts `quota_exhausted`. `getProviderOverallStatus` returns `'Partially Degraded'` when present. |
| FIND-020: Add to group disabled | ⚠️ Fixed with issues | Button is functional now but has 3 new bugs (see below). |
| FIND-021: AppImage sidecar path | ❌ Not fixed | Same root cause as FIND-013 — naming mismatch between build output and runtime lookup. |
| FIND-022: Third DB connection on reload | ✅ Fixed | No `init_db()` call in `handle_internal_config_reload`. Comment notes scheduler will re-sync. |
| FIND-023: calculate_cost partial pricing | ✅ Fixed | Input and output costs calculated independently and summed. Test confirms. |

---

## New Bugs Introduced by Fixes

| ID | Severity | File(s) | Description |
|---|---|---|---|
| NEW-001 | Medium | `src-tauri/src/commands.rs:418` | **`get_usage_by_group` hardcodes `reset_hour = 0`** — The query now accepts `reset_hour` but the Tauri command always passes `0`. For providers with `quota_reset_utc_hour != 0`, group usage time window is misaligned. |
| NEW-002 | Medium | `sidecar/src/proxy/server.rs:1089` | **Daily totals not reloaded after config reload** — In-memory daily counters preserved from old state but never refreshed from DB. The scheduler only resets counters at midnight, never reloads from DB. If sidecar was restarted and lost state, config reload won't restore daily totals. |
| NEW-003 | Low | `sidecar/src/proxy/server.rs:626` | **429 response body read has no timeout** — Unlike other body reads (wrapped in 120s timeout), the 429 body discard can hang indefinitely if upstream stalls after sending headers. |
| NEW-004 | Low | `src-tauri/src/commands.rs:462` | **`build_entry_statuses` uses blocking HTTP on Tauri thread** — `reqwest::blocking::Client` with 2s timeout blocks the Tauri event thread. Could cause UI stutter. |
| NEW-005 | Low | `sidecar/src/proxy/server.rs:999-1006` | **`handle_internal_router_set_entry` doesn't notify scheduler** — Groups/providers reloaded from disk but scheduler's own `Arc<RwLock<Arc<Vec<Group>>>>` clone is not notified. Scheduler uses stale group data until next config reload. |
| NEW-006 | Medium | `src/hooks/useMetricsStream.ts:19` | **EventSource URL still defaults to `localhost`** — Same IPv6 resolution issue as FIND-011. If proxy binds to `127.0.0.1` only and `localhost` resolves to `::1`, SSE metrics stream fails. |
| NEW-007 | Medium | `src/pages/Providers.tsx:402-422` | **"Add to group" doesn't refresh store after save** — `handleSelectGroup` saves the group via IPC but never calls `refreshGroups()` or updates zustand store. ModelGroups page won't see the new entry until full page reload. |
| NEW-008 | Medium | `src/pages/Providers.tsx:418` | **`handleSelectGroup` silently swallows errors** — Empty `catch` block. If `saveGroup` fails, user gets zero feedback and dropdown closes, making it appear successful. |

---

## Summary

| Category | Count |
|---|---|
| Fixed correctly | 14 |
| Fixed with issues | 2 |
| Not fixed | 2 |
| New bugs introduced | 8 |

### New Bugs by Severity

| Severity | Count |
|---|---|
| Medium | 4 |
| Low | 3 |
| **Total** | **7** |

---

## Recommended Priority Order for Fixes

1. **NEW-001** — Pass actual `reset_hour` to `get_usage_by_group` in Tauri command (derive from providers)
2. **NEW-007** — Refresh zustand store after "Add to group" save
3. **NEW-008** — Add error toast in `handleSelectGroup` catch block
4. **NEW-006** — Change `useMetricsStream.ts` default from `localhost` to `127.0.0.1`
5. **FIND-013/021** — Fix sidecar binary naming (remove suffix from `build.sh` or update `tauri.conf.json`)
6. **FIND-017** — Use UTC-based date inputs in UsageMetrics
7. **NEW-002** — Reload daily totals from DB after config reload
8. **NEW-003** — Wrap 429 body read in timeout
9. **NEW-004** — Use async HTTP client in `build_entry_statuses`
10. **NEW-005** — Notify scheduler of entry changes in `handle_internal_router_set_entry`

---

## Design Gaps Status (from Audit 006)

| Gap | Status | Notes |
|-----|--------|-------|
| DG-001: `daily_request_quota` not enforced | ✅ Addressed | `select_entry` now checks it, `/v1/models` filters on it |
| DG-002: Latency timeout cooldown configurable | ✅ Addressed | UI input + backend field with serde rename |
| DG-003: Consecutive errors cooldown configurable | ✅ Addressed | UI input + backend field with serde rename |
| DG-005: `opencode.json` cached config | ✅ Addressed | `save_opencode_cache()` / `load_opencode_cache()` implemented |
| DG-006: System agents missing | ✅ Addressed | All 7 agents in backend struct and frontend UI |
| DG-007: No streaming metrics display | ⚠️ Partially | `useMetricsStream` hook exists but **never imported or used** in any page component. No real-time token visualization UI. |
| DG-008: No 404 route | ✅ Addressed | Catch-all route with `NotFound` component |
| DG-009: Add to group button | ✅ Addressed | Button functional, dropdown of groups, saves via IPC |

**7 of 8 design gaps fully addressed. DG-007 is the only remaining gap.**

---

## Notes

- 116 Rust tests pass (up from 106 in Audit 006). TypeScript check is clean.
- The counter divergence fix (FIND-001) is the most significant improvement — quota enforcement now works correctly across multiple groups.
- The sidecar binary naming issue (FIND-013/021) remains the only blocker for AppImage distribution.
- This is Audit 007 — a fix verification audit.
