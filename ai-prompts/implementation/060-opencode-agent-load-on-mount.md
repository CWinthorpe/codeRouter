# Feat-060: Read Current Agent Model Assignments on OpenCode Setup Tab

## Problem

The OpenCode Setup tab always blanks out agent model assignments when navigating away and back. The dropdowns reset to "use default" even though `opencode.json` has `coderouter/<alias>` values written. There is no read path — agent assignments are write-only.

## Fix Requirements

### 1. Add `get_current_agent_mapping()` in `sidecar/src/opencode/config_writer.rs`

Add a new public function that reads the opencode config file and extracts current `coderouter/` agent model assignments:

- Read the opencode config JSON using the existing private `read_config()` function
- For each agent key (`build`, `plan`, `general`, `explore`, `compaction`, `title`, `summary`): check `config.agent.<key>.model` — if it starts with `"coderouter/"`, extract the alias (strip the prefix)
- For `small_model`: check `config.small_model` — if it starts with `"coderouter/"`, extract the alias
- Return an `AgentMapping` struct with the extracted values (or `None` for each unset/non-coderouter field)
- Use `resolve_opencode_config_path()` to find the config file

### 2. Add `get_opencode_agent_models` Tauri command in `src-tauri/src/commands.rs`

- Call `config_writer::get_current_agent_mapping(config_path)`
- Convert the internal `AgentMapping` to the IPC `OpenCodeAgentMapping` struct (same pattern as existing commands)
- Register in `main.rs` alongside the other opencode commands

### 3. Add `getOpencodeAgentModels()` IPC function in `src/lib/ipc.ts`

- New async function that invokes `get_opencode_agent_models`
- Returns `OpenCodeAgentMapping`

### 4. Update `OpenCodeSetup.tsx` to load current assignments on mount

- Add a `useEffect` that calls `getOpencodeAgentModels()` on component mount
- When the response comes back, merge it into the `mapping` state (only overwrite null fields, or just set the whole mapping)
- This should happen alongside the existing mount logic that checks provider status (around line 132-143)
- The dropdowns should show the currently-assigned group alias, or "use default" if null

### 5. Tests

- Add a test in `config_writer.rs` for `get_current_agent_mapping()`:
  - Config with coderouter agent assignments → returns correct mapping
  - Config with no coderouter assignments → returns all-null mapping
  - Config with mix of coderouter and non-coderouter assignments → only extracts coderouter ones
  - Config with small_model set to coderouter alias → extracts correctly

### Files to modify
- `sidecar/src/opencode/config_writer.rs` — add `get_current_agent_mapping()`
- `src-tauri/src/commands.rs` — add `get_opencode_agent_models` command
- `src-tauri/src/main.rs` — register new command
- `src/lib/ipc.ts` — add `getOpencodeAgentModels()`
- `src/pages/OpenCodeSetup.tsx` — load on mount

### Build & Test Commands
```bash
PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig cargo test -p coderouter-proxy
PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig cargo build
npx tsc --noEmit
```
