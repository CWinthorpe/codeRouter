#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")" && pwd)"
TAURI_DIR="$PROJECT_ROOT/src-tauri"
SIDECAR_DIR="$PROJECT_ROOT/sidecar"
BINARIES_DIR="$TAURI_DIR/sidecar"

BUILD_GUI=true
BUILD_TUI=false

for arg in "$@"; do
    case "$arg" in
        --tui)
            BUILD_TUI=true
            ;;
        --tui-only)
            BUILD_GUI=false
            BUILD_TUI=true
            ;;
    esac
done

# Detect target triple from rustc
TARGET="$(rustc -vV | grep host | cut -d' ' -f2)"
SIDECAR_BIN="coderouter-proxy"
SIDECAR_TRIPLE="${SIDECAR_BIN}-${TARGET}"
TUI_BIN="coderouter-tui"

# Check for required dependencies
check_dep() {
  if ! command -v "$1" &>/dev/null; then
    echo "Error: '$1' is required but not installed." >&2
    exit 1
  fi
}

check_dep cargo
check_dep rustc
check_dep npm

if [ "$BUILD_TUI" = true ]; then
    echo "=== Building TUI binary (release) ==="
    cd "$PROJECT_ROOT"
    cargo build --release -p coderouter-tui
    echo "TUI build complete: target/release/${TUI_BIN}"
fi

if [ "$BUILD_GUI" = true ]; then
    echo "=== Step 1: Building sidecar binary (release) ==="
    cd "$PROJECT_ROOT"
    cargo build --release --target "$TARGET" -p coderouter-proxy
    echo "Sidecar build complete."

    echo "=== Step 2: Copying Sidecar to tauri binaries ==="
    mkdir -p "$BINARIES_DIR"
    cp "$PROJECT_ROOT/target/$TARGET/release/$SIDECAR_BIN" "$BINARIES_DIR/$SIDECAR_TRIPLE"
    chmod +x "$BINARIES_DIR/$SIDECAR_TRIPLE"
    echo "Sidecar binary copied to $BINARIES_DIR/$SIDECAR_TRIPLE"

    echo "=== Step 3: Running tauri build ==="
    cd "$PROJECT_ROOT"
    if [ -d "/usr/lib/x86_64-linux-gnu/pkgconfig" ]; then
      export PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig
    elif [ -d "/usr/lib/aarch64-linux-gnu/pkgconfig" ]; then
      export PKG_CONFIG_PATH=/usr/lib/aarch64-linux-gnu/pkgconfig
    elif [ -d "/usr/lib64/pkgconfig" ]; then
      export PKG_CONFIG_PATH=/usr/lib64/pkgconfig
    elif [ -d "/usr/lib/pkgconfig" ]; then
      export PKG_CONFIG_PATH=/usr/lib/pkgconfig
    fi
    if [ ! -d "node_modules" ] || [ "package.json" -nt "node_modules" ]; then
        npm install
    fi
    npm run tauri build

    echo ""
    echo "=== Step 4: Signing AppImage ==="
    if [ -f "$TAURI_DIR/update.key" ]; then
      export TAURI_SIGNING_PRIVATE_KEY_PATH="$TAURI_DIR/update.key"
      APPIMAGE="$(find "$PROJECT_ROOT/target/release/bundle/appimage/" -name "*.AppImage" 2>/dev/null | head -1)"
      if [ -n "$APPIMAGE" ]; then
        echo "Signing $APPIMAGE..."
        cargo tauri signer sign "$APPIMAGE"
        echo "Signing complete."
      else
        echo "Warning: No AppImage found to sign."
      fi
    else
      echo "Warning: No update.key found — skipping signing. Generate one with: cargo tauri signer generate -w ./update.key -p ''"
    fi

    echo ""
    echo "=== GUI Build complete ==="
    echo "AppImage output:"
    find "$PROJECT_ROOT/target/release/bundle/appimage/" -name "*.AppImage" 2>/dev/null || echo "No AppImage found — check build logs above."
fi

if [ "$BUILD_TUI" = true ]; then
    echo ""
    echo "=== TUI Build output ==="
    echo "Binary: $PROJECT_ROOT/target/release/${TUI_BIN}"
fi

echo ""
echo "=== Build finished ==="
