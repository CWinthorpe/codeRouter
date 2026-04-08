# CodeRouter - Project Plan

## Overview

CodeRouter is a Linux desktop application (distributed as AppImage) that acts as a local OpenAI-compatible proxy router. It sits between AI coding tools like OpenCode and multiple upstream LLM providers, providing intelligent failover, cost management, model grouping, and seamless OpenCode configuration integration.

The app is built with **Tauri** (Rust backend sidecar + React/TypeScript frontend) and exposes a local OpenAI-compatible HTTP API on port **4141**.

---

## Core Goals

1. Aggregate multiple OpenAI-compatible and Anthropic-compatible upstream providers behind a single local endpoint.
2. Group models across providers with configurable priority and automatic failover.
3. Advertise model groups as virtual models on the local proxy endpoint.
4. Integrate directly with OpenCode config to auto-configure agents and providers.
5. Track per-provider usage, costs, and health to drive intelligent routing decisions.

---

## Tech Stack

| Layer | Technology |
|---|---|
| Desktop shell | Tauri 2.x (AppImage target) |
| Proxy service | Rust (Axum HTTP server, runs as Tauri sidecar) |
| Frontend UI | React 18 + TypeScript + Vite |
| UI component lib | shadcn/ui + Tailwind CSS |
| Config storage | JSON files (`~/.config/coderouter/`) |
| Credential storage | Linux Secret Service via `libsecret` (system keychain) |
| Usage/metrics DB | SQLite (via `rusqlite`) embedded in sidecar |
| System tray | Tauri tray plugin |


---

## Provider Support

### Supported Protocol Types

CodeRouter supports two upstream provider protocol types and normalizes all traffic to OpenAI-compatible format before passing responses back to the client:

| Protocol | Description |
|---|---|
| **OpenAI-compatible** | Any provider with a `/v1/chat/completions` and `/v1/models` endpoint following the OpenAI API spec |
| **Anthropic-compatible** | Providers using the Anthropic Messages API (`/v1/messages`) with `x-api-key` auth |

The proxy layer translates:
- Anthropic request format → OpenAI request format (before forwarding to provider)
- Anthropic response format → OpenAI response format (before returning to client)
- Anthropic streaming SSE → OpenAI streaming SSE chunks

This means clients (e.g. OpenCode) always see a pure OpenAI-compatible API regardless of what upstream providers are configured.

### Adding a Provider

When a user adds a provider they specify:
- Display name
- Base URL
- Protocol type (OpenAI-compatible or Anthropic-compatible)
- API key (stored in system keychain, referenced by ID in config)
- Optional: daily token quota, daily request quota, quota reset time (UTC hour)
- Optional: per-model overrides (e.g. custom model name mapping)

### Model Discovery

On provider add (and on a daily refresh schedule), CodeRouter will:
1. Call `GET /v1/models` (OpenAI) or equivalent discovery endpoint (Anthropic) to list available models.
2. For each model, attempt to retrieve metadata: context window size, max output tokens, input/output cost per million tokens.
3. Store model list and metadata in the provider's JSON config file.
4. Surface the fetched models in the UI for the user to browse and add to their active model list.

Model metadata sources in priority order:
1. Provider's `/v1/models` response (if it includes pricing/context fields)
2. OpenAI-style model detail endpoint `GET /v1/models/{model}` if available
3. Manual override entered by the user in the UI


---

## Model Groups

### Concept

A **model group** is a named virtual model that maps to one or more upstream provider+model pairs, each with a priority order. The group is advertised as a single model ID on the local proxy endpoint.

Example: a group named `glm-5-router` might contain:
1. Z.AI Coding Plan Account A → `glm-4.5` (priority 1, highest)
2. Z.AI Coding Plan Account B → `glm-4.5` (priority 2)
3. Z.AI Pay-As-You-Go → `glm-4.5` (priority 3, fallback)

When a request comes in for `glm-5-router`, CodeRouter routes to priority 1. If that provider is unavailable or exhausted, it falls over to priority 2, and so on — completely transparently to the client.

### Group Configuration

Each group has:
- **Alias** (the model ID advertised to clients, e.g. `glm-5-router`)
- **Display name** (shown in UI)
- **Provider entries** (ordered list, each with: provider ID, upstream model name, optional per-entry quota override)
- **Failover settings** (which triggers are enabled, configurable per group)

### Advertised on Proxy Endpoint

`GET /v1/models` on the local proxy returns only the group aliases (not raw upstream model names). Each alias appears as a standard OpenAI model object with metadata derived from the highest-priority active provider in the group.


---

## Failover & Recovery Logic

### Failover Triggers

All triggers are configurable per group (on/off toggles + threshold values):

| Trigger | Description |
|---|---|
| **HTTP 429 / rate limit** | Immediately failover when provider returns a rate limit error |
| **Daily quota exhausted** | Failover when tracked usage reaches the configured daily quota for that provider entry |
| **N consecutive errors** | Failover after N consecutive non-429 errors (default: 5, configurable) |
| **Latency timeout** | Failover if a provider does not respond within a configured timeout (default: 30s) |
| **Manual disable** | User can force-disable a provider entry in the UI |

### Recovery Strategy

Recovery behavior depends on how the failover was triggered:

| Cause | Recovery Method |
|---|---|
| Daily quota exhausted | Re-enable automatically at the provider's configured quota reset time (UTC hour, set per provider). Defaults to 00:00 UTC if not specified. |
| HTTP 429 rate limit | Exponential backoff cooldown (starting at 60s, doubling up to 1 hour max), then probe-based re-enable: send a lightweight test request; if it succeeds, re-enable the provider. |
| Consecutive errors | Fixed cooldown period (configurable, default 10 minutes) + probe-based re-enable. |
| Latency timeout | Cooldown period (configurable, default 5 minutes) + probe-based re-enable. |
| Manual disable | Manual re-enable only (user toggles in UI). |

### Request Routing Flow

```
Incoming request for model alias "glm-5-router"
  → Look up group config
  → Filter provider list: remove manually disabled and currently-in-cooldown entries
  → Select highest-priority active entry
  → Translate request format if needed (Anthropic ↔ OpenAI)
  → Forward request to upstream provider
  → On success: record usage metrics, return translated response to client
  → On failure: record error, apply failover trigger logic, retry with next priority entry
  → If all entries exhausted: return 503 with error detail
```


---

## Usage Tracking & Metrics

### What is Tracked (per provider+model entry)

- Request count (total, success, error, by error type)
- Input tokens used
- Output tokens used
- Estimated cost (input tokens × input price + output tokens × output price)
- Daily rolling totals (reset at provider's configured quota reset time)
- Latency percentiles (p50, p95) per day

All metrics are stored in a local SQLite database at `~/.local/share/coderouter/metrics.db`.

### UI Dashboard

The UI will display:
- Per-provider daily usage bars (tokens used vs. quota, cost today)
- Per-group health status (which provider is currently active, which are in cooldown)
- Recent request log (timestamp, group, provider used, tokens, latency, status)
- Historical cost charts (last 7/30 days per provider)

---

## Local Proxy API

The sidecar exposes a standard OpenAI-compatible REST API on `http://localhost:4141`.

### Endpoints

| Endpoint | Method | Description |
|---|---|---|
| `/v1/models` | GET | Returns list of all enabled model groups as OpenAI model objects |
| `/v1/chat/completions` | POST | Chat completion (streaming and non-streaming) routed through failover logic |
| `/v1/completions` | POST | Legacy completions endpoint (forwarded as-is if provider supports it) |
| `/health` | GET | Returns proxy status, active providers, current failover states |

### Authentication

The local proxy accepts requests with no auth key or with any API key value — since it is local-only (`127.0.0.1`), authentication is not required. The proxy itself handles upstream authentication internally.


---

## OpenCode Integration

### Overview

CodeRouter can automatically configure OpenCode to use it as a provider. This is done by writing to the OpenCode global config file at `~/.config/opencode/opencode.json`. The app uses surgical JSON merge/patch to only update the `provider` and `agent` sections, preserving all other user settings.

### Basic Auto-Setup (Proxy Provider Only)

Without using any agent mapping feature, the user can click "Configure OpenCode" in the UI. This will:

1. Detect the OpenCode global config path (`~/.config/opencode/opencode.json`).
2. Add/update a custom provider entry pointing to CodeRouter:

```json
{
  "provider": {
    "coderouter": {
      "npm": "@ai-sdk/openai-compatible",
      "name": "CodeRouter",
      "options": {
        "baseURL": "http://localhost:4141/v1",
        "apiKey": "coderouter"
      },
      "models": {
        "<group-alias>": {
          "name": "<group display name>",
          "limit": {
            "context": <context_window>,
            "output": <max_output_tokens>
          }
        }
      }
    }
  }
}
```

All active model groups are injected as models under the `coderouter` provider.

### Agent Mapping Feature

This optional feature allows users to map model groups to specific OpenCode built-in agents, so each agent type uses the best-suited model.

#### OpenCode Built-in Agents (configurable)

| Agent | Mode | Purpose |
|---|---|---|
| `build` | primary | Main development agent, all tools enabled |
| `plan` | primary | Planning/analysis, restricted tools (no file writes) |
| `general` | subagent | General-purpose multi-step tasks |
| `explore` | subagent | Read-only codebase exploration |
| `compaction` | system | Auto context compaction (hidden) |
| `title` | system | Auto session title generation (hidden) |
| `summary` | system | Auto session summary (hidden) |

The UI will show a configuration panel with a dropdown per configurable agent (`build`, `plan`, `general`, `explore`) allowing the user to select which model group to assign.

When the user applies the configuration, CodeRouter writes to the OpenCode config:

```json
{
  "agent": {
    "build": {
      "model": "coderouter/glm-5-router"
    },
    "plan": {
      "model": "coderouter/fast-model-router"
    },
    "general": {
      "model": "coderouter/glm-5-router"
    },
    "explore": {
      "model": "coderouter/fast-model-router"
    }
  }
}
```

The model ID format follows OpenCode's `provider/model-id` convention.

#### Small Model

The UI also allows selecting a model group for OpenCode's `small_model` setting (used for lightweight tasks like session title generation), which writes:

```json
{
  "small_model": "coderouter/fast-model-router"
}
```


---

## Configuration Files

All config lives under `~/.config/coderouter/`. The directory structure:

```
~/.config/coderouter/
  config.json          # App-level settings (proxy port, UI prefs)
  providers.json       # List of configured upstream providers
  groups.json          # Model group definitions and priority lists
  opencode.json        # Cached OpenCode integration settings

~/.local/share/coderouter/
  metrics.db           # SQLite usage/metrics database
```

API keys are NOT stored in these files. They are stored in the Linux Secret Service (libsecret) under the service name `coderouter` with the provider ID as the attribute key. The `providers.json` file contains only a reference to the keychain entry by provider ID.

### providers.json schema (example)

```json
[
  {
    "id": "zai-coding-a",
    "name": "Z.AI Coding Plan - Account A",
    "protocol": "openai",
    "baseUrl": "https://api.z.ai/v1",
    "credentialKey": "zai-coding-a",
    "dailyTokenQuota": 1000000,
    "quotaResetUtcHour": 0,
    "enabled": true,
    "models": [
      {
        "id": "glm-4.5",
        "contextWindow": 128000,
        "maxOutputTokens": 8192,
        "inputCostPer1M": 0,
        "outputCostPer1M": 0,
        "lastRefreshed": "2026-04-07T00:00:00Z"
      }
    ]
  },
  {
    "id": "anthropic-main",
    "name": "Anthropic API",
    "protocol": "anthropic",
    "baseUrl": "https://api.anthropic.com",
    "credentialKey": "anthropic-main",
    "dailyTokenQuota": null,
    "quotaResetUtcHour": 0,
    "enabled": true,
    "models": [
      {
        "id": "claude-sonnet-4-20250514",
        "contextWindow": 200000,
        "maxOutputTokens": 64000,
        "inputCostPer1M": 3.0,
        "outputCostPer1M": 15.0,
        "lastRefreshed": "2026-04-07T00:00:00Z"
      }
    ]
  }
]
```

### groups.json schema (example)

```json
[
  {
    "id": "glm-5-router",
    "alias": "glm-5-router",
    "displayName": "GLM-5 (Multi-Account)",
    "entries": [
      {
        "providerId": "zai-coding-a",
        "modelId": "glm-4.5",
        "priority": 1,
        "dailyTokenQuotaOverride": null,
        "enabled": true,
        "status": "active",
        "cooldownUntil": null
      },
      {
        "providerId": "zai-coding-b",
        "modelId": "glm-4.5",
        "priority": 2,
        "dailyTokenQuotaOverride": null,
        "enabled": true,
        "status": "active",
        "cooldownUntil": null
      },
      {
        "providerId": "zai-payg",
        "modelId": "glm-4.5",
        "priority": 3,
        "dailyTokenQuotaOverride": null,
        "enabled": true,
        "status": "active",
        "cooldownUntil": null
      }
    ],
    "failoverConfig": {
      "on429": true,
      "onQuotaExhausted": true,
      "onConsecutiveErrors": true,
      "consecutiveErrorThreshold": 5,
      "onLatencyTimeout": true,
      "latencyTimeoutMs": 30000
    }
  }
]
```


---

## UI Structure

### Application Shell

- **System tray icon**: green (proxy running) / red (proxy stopped). Left-click opens main window. Right-click menu: Open, Start/Stop Proxy, Configure OpenCode, Quit.
- **Main window**: single-page app with sidebar navigation.

### Pages / Sections

#### 1. Dashboard
- Proxy status (running/stopped, port, uptime)
- Per-provider health overview cards (active/cooldown/disabled, tokens used today, estimated cost today)
- Recent request feed (last 20 requests: timestamp, group used, provider used, tokens, latency, status)
- Quick action: "Configure OpenCode" button

#### 2. Providers
- List of configured upstream providers with status badge
- Add provider form:
  - Name, Base URL, Protocol type (OpenAI-compatible / Anthropic-compatible)
  - API key field (written to keychain on save)
  - Daily token quota (optional), quota reset UTC hour
- Per-provider actions: Edit, Test Connection, Refresh Models, Delete
- Model browser: expandable list of models fetched from provider with metadata

#### 3. Model Groups
- List of all defined groups with their alias, entry count, current active provider
- Create/edit group:
  - Set alias and display name
  - Add provider+model entries (searchable dropdown of providers and their models)
  - Drag-and-drop reorder to set priority
  - Per-entry: enable/disable toggle, quota override
  - Failover settings panel (toggles per trigger type + threshold inputs)
- Group status panel: live view showing which entry is currently active and any entries in cooldown (with countdown timer)

#### 4. OpenCode Setup
- Detected OpenCode config path (editable)
- Toggle: "Enable CodeRouter as OpenCode provider"
- Agent mapping section:
  - Dropdowns for: build agent, plan agent, general subagent, explore subagent
  - small_model selector
- Preview of the JSON that will be written to OpenCode config
- "Apply Configuration" button
- "Remove CodeRouter from OpenCode config" button

#### 5. Usage & Metrics
- Date range picker
- Per-provider cost and token usage bar charts
- Per-group request volume chart
- Tabular request log with filtering (by provider, group, status, date range)
- Export to CSV button

#### 6. Settings
- Proxy port (default 4141)
- Proxy listen address (default 127.0.0.1)
- Model metadata auto-refresh interval (default: daily)
- Log verbosity
- Reset all data option


---

## Proxy Sidecar Architecture (Rust)

The proxy runs as a Rust binary sidecar managed by the Tauri process. It communicates with the Tauri frontend via:
- **IPC commands** (Tauri's invoke mechanism): for config reads/writes, status queries
- **Local HTTP** (on a second internal port, e.g. 4142): for metrics/log streaming to the UI

### Key Rust Modules

| Module | Responsibility |
|---|---|
| `proxy::server` | Axum HTTP server on port 4141, request parsing, response streaming |
| `proxy::router` | Group lookup, priority selection, failover state machine |
| `proxy::translator` | OpenAI ↔ Anthropic request/response format translation |
| `proxy::upstream` | HTTP client pool, upstream request dispatch, timeout handling |
| `config::store` | JSON config file read/write with file-lock safety |
| `credentials::keychain` | libsecret integration for API key storage/retrieval |
| `metrics::recorder` | SQLite writes for request events, token counts, costs |
| `metrics::scheduler` | Daily quota reset timer, probe-based recovery scheduler |
| `models::refresher` | Periodic upstream model metadata fetch and cache update |

### Protocol Translation Detail

**OpenAI → Anthropic (for Anthropic-protocol providers):**
- Map `messages` array roles: `system` messages move to Anthropic's top-level `system` field
- Map `max_tokens` → Anthropic's `max_tokens`
- Map `stream: true` → Anthropic's `stream: true`
- Map `model` to the configured upstream model ID
- Set `anthropic-version` header and `x-api-key` from keychain

**Anthropic → OpenAI (response translation back to client):**
- Map `content[].text` → `choices[0].message.content`
- Map `usage.input_tokens` / `usage.output_tokens` → `usage.prompt_tokens` / `usage.completion_tokens`
- Map streaming `content_block_delta` events → OpenAI `data: {"choices": [{"delta": {"content": "..."}}]}` SSE chunks
- Map stop reason `end_turn` → `finish_reason: "stop"`

---

## AppImage Packaging

- Tauri's `tauri build` with `appimage` target produces a self-contained `.AppImage`
- The Rust proxy sidecar binary is bundled as a Tauri sidecar resource
- The AppImage includes all frontend assets and the sidecar binary
- On first launch, the app creates `~/.config/coderouter/` and `~/.local/share/coderouter/` if they do not exist
- The system tray integration uses the desktop environment's tray API via Tauri's tray plugin (works on GNOME with AppIndicator extension, KDE Plasma, XFCE, etc.)

---

## Development Phases

### Phase 1 — Core Proxy
- Rust sidecar: Axum server, config loading, OpenAI passthrough (no failover yet)
- Single provider add/test flow in UI
- Basic `/v1/models` and `/v1/chat/completions` passthrough
- Anthropic protocol translation layer

### Phase 2 — Provider Management
- Full provider CRUD in UI
- Model discovery and metadata fetch
- Keychain credential storage
- Model browser in UI

### Phase 3 — Model Groups & Failover
- Group creation/editing UI
- Priority-based routing in proxy router
- All failover triggers implemented
- Cooldown + recovery scheduler
- Group status live view in UI

### Phase 4 — Usage Tracking
- SQLite metrics recording
- Per-provider daily quota enforcement
- Dashboard usage cards and recent request feed
- Usage & Metrics page with charts

### Phase 5 — OpenCode Integration
- OpenCode config detection and JSON merge/patch
- Basic auto-setup (provider injection)
- Agent mapping UI and config writing
- small_model selector

### Phase 6 — Polish & Packaging
- System tray icon with status
- AppImage build pipeline
- Settings page
- Error handling and user-facing error messages
- First-run onboarding flow

---

## Open Questions / Future Considerations

- **Request caching**: optionally cache identical requests to reduce upstream calls and cost
- **Load balancing mode**: round-robin across equal-priority providers rather than strict failover (useful for spreading load across multiple unlimited accounts)
- **Rate limiting the local endpoint**: prevent runaway client requests from burning through quotas
- **Multiple OpenCode project configs**: ability to write different agent mappings per project (not just global config)
- **Windows/macOS support**: Tauri supports both; the main gap is credential storage (replace libsecret with OS-native keychain)
- **Provider preset library**: built-in presets for common providers (OpenAI, Anthropic, Z.AI, Groq, DeepSeek, etc.) with known base URLs and model lists to speed up setup
