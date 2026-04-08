# CodeRouter

A Linux desktop application that acts as a local OpenAI-compatible proxy router. Sits between AI coding tools (like OpenCode) and multiple upstream LLM providers, providing intelligent failover, cost management, model grouping, and seamless OpenCode configuration integration.

Built with **Tauri 2.x** (Rust sidecar + React/TypeScript frontend) and distributed as an **AppImage**.

## Features

- **Multi-Provider Aggregation** — Add multiple OpenAI-compatible and Anthropic-compatible providers behind a single local endpoint (`localhost:4141`)
- **Model Groups & Failover** — Group models across providers with priority ordering. Automatic failover on 429 errors, quota exhaustion, consecutive errors, or latency timeouts
- **Protocol Translation** — Transparent Anthropic ↔ OpenAI translation. Clients always see an OpenAI-compatible API
- **Usage Tracking** — SQLite-backed metrics: per-provider costs, token usage, latency, request logs with charts and CSV export
- **OpenCode Integration** — Auto-configure OpenCode to use CodeRouter as a provider, with per-agent model mapping
- **System Tray** — Status indicator, quick start/stop proxy, hide-to-tray on window close

## Tech Stack

| Layer | Technology |
|---|---|
| Desktop shell | Tauri 2.x (AppImage target) |
| Proxy service | Rust (Axum HTTP server, Tauri sidecar) |
| Frontend UI | React 18 + TypeScript + Vite |
| UI components | shadcn/ui + Tailwind CSS |
| Config storage | JSON files (`~/.config/coderouter/`) |
| Credential storage | Linux Secret Service via `libsecret` |
| Metrics DB | SQLite (`rusqlite`, bundled) |

## System Requirements

- Linux (tested on Linux Mint)
- GTK3, WebKit2GTK (standard on most desktop distros)
- `libayatana-appindicator3-1` (system tray support)

## Quick Start

### Running from Source

```bash
# Install system dependencies (Debian/Ubuntu)
sudo apt-get install -y libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev

# Clone and build
git clone https://github.com/CWinthorpe/codeRouter.git
cd codeRouter
npm install

# Development mode
make dev

# Production build (AppImage)
make build
```

The AppImage will be produced at `target/release/bundle/appimage/CodeRouter_0.1.0_amd64.AppImage`.

### Using the AppImage

```bash
chmod +x CodeRouter_0.1.0_amd64.AppImage
./CodeRouter_0.1.0_amd64.AppImage
```

On first launch, CodeRouter creates `~/.config/coderouter/` and `~/.local/share/coderouter/`.

## Proxy API

The sidecar exposes a standard OpenAI-compatible REST API on `http://localhost:4141`:

| Endpoint | Method | Description |
|---|---|---|
| `/v1/models` | GET | List all enabled model groups |
| `/v1/chat/completions` | POST | Chat completion (streaming + non-streaming) |
| `/v1/completions` | POST | Legacy completions |
| `/health` | GET | Proxy status and uptime |

No authentication required — the proxy handles upstream auth internally.

### Example Usage

```bash
# Check health
curl http://localhost:4141/health

# List available model groups
curl http://localhost:4141/v1/models

# Chat completion (non-streaming)
curl http://localhost:4141/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "glm-5-router", "messages": [{"role": "user", "content": "Hello"}]}'

# Chat completion (streaming)
curl http://localhost:4141/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "glm-5-router", "messages": [{"role": "user", "content": "Hello"}], "stream": true}'
```

## Configuration

Config files live at `~/.config/coderouter/`:

```
~/.config/coderouter/
  config.json          # App settings (port, host, refresh interval, log verbosity)
  providers.json       # Upstream provider configs
  groups.json          # Model group definitions
  opencode.json        # Cached OpenCode integration settings

~/.local/share/coderouter/
  metrics.db           # SQLite usage/metrics database
  proxy.log            # Sidecar log file
```

API keys are stored in the Linux Secret Service (libsecret), never in config files.

## Development

```bash
# Run all tests
make test          # cargo test --workspace

# TypeScript check
npx tsc --noEmit

# Build AppImage
make build         # runs ./build.sh

# Dev mode with hot reload
make dev           # npm run tauri dev
```

### Project Structure

```
codeRouter/
├── sidecar/              # Rust proxy binary
│   └── src/
│       ├── config/       # JSON config store + serde models
│       ├── credentials/  # libsecret keychain wrapper
│       ├── metrics/      # SQLite recorder, queries, scheduler
│       ├── models/       # Upstream model discovery
│       ├── opencode/     # OpenCode config writer
│       └── proxy/        # Axum server, router, protocol translator
├── src-tauri/            # Tauri desktop shell
│   └── src/
│       ├── commands.rs   # All Tauri IPC commands
│       └── main.rs       # Entry point, sidecar lifecycle, tray
├── src/                  # React frontend
│   ├── components/       # AppShell (sidebar layout)
│   ├── pages/            # Dashboard, Providers, Groups, etc.
│   ├── store/            # Zustand global state
│   ├── lib/ipc.ts        # Typed IPC wrapper
│   └── types/index.ts    # Shared TypeScript types
├── build.sh              # AppImage build script
└── Makefile              # build / dev / test targets
```

## License

MIT
