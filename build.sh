#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")" && pwd)"
TAURI_DIR="$PROJECT_ROOT/src-tauri"
SIDECAR_DIR="$PROJECT_ROOT/sidecar"
BINARIES_DIR="$TAURI_DIR/sidecar"

TARGET="x86_64-unknown-linux-gnu"
SIDECAR_BIN="coderouter-proxy"
SIDECAR_TRIPLE="${SIDECAR_BIN}-${TARGET}"

echo "=== Step 1: Building sidecar binary (release) ==="
cd "$PROJECT_ROOT"
cargo build --release -p coderouter-proxy
echo "Sidecar build complete."

echo "=== Step 2: Copying Sidecar to tauri binaries ==="
mkdir -p "$BINARIES_DIR"
cp "$PROJECT_ROOT/target/release/$SIDECAR_BIN" "$BINARIES_DIR/$SIDECAR_TRIPLE"
chmod +x "$BINARIES_DIR/$SIDECAR_TRIPLE"
echo "Sidecar binary copied to $BINARIES_DIR/$SIDECAR_TRIPLE"

echo "=== Step 3: Running tauri build ==="
cd "$PROJECT_ROOT"
export PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig
npm run tauri build

echo ""
echo "=== Build complete ==="
echo "AppImage output:"
find "$PROJECT_ROOT/target/release/bundle/appimage/" -name "*.AppImage" 2>/dev/null || echo "No AppImage found — check build logs above."
