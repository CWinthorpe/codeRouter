#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")" && pwd)"
TAURI_DIR="$PROJECT_ROOT/src-tauri"
SIDECAR_DIR="$PROJECT_ROOT/sidecar"
BINARIES_DIR="$TAURI_DIR/sidecar"

BUILD_GUI=true
BUILD_TUI=false
BUILD_RELEASE=false

for arg in "$@"; do
    case "$arg" in
        --tui)
            BUILD_TUI=true
            ;;
        --tui-only)
            BUILD_GUI=false
            BUILD_TUI=true
            ;;
        --release)
            BUILD_RELEASE=true
            BUILD_GUI=true
            BUILD_TUI=true
            ;;
    esac
done

TARGET="$(rustc -vV | grep host | cut -d' ' -f2)"
SIDECAR_BIN="coderouter-proxy"
SIDECAR_TRIPLE="${SIDECAR_BIN}-${TARGET}"
TUI_BIN="coderouter-tui"
SIGN_KEY="$TAURI_DIR/update.key"

if [ "$BUILD_RELEASE" = true ]; then
    VERSION=$(grep '"version"' "$TAURI_DIR/tauri.conf.json" | head -1 | sed 's/.*: *"\([^"]*\)".*/\1/')
    echo "=== Release build for version: ${VERSION} ==="
fi

check_dep() {
  if ! command -v "$1" &>/dev/null; then
    echo "Error: '$1' is required but not installed." >&2
    exit 1
  fi
}

check_dep cargo
check_dep rustc
check_dep npm

if [ "$BUILD_GUI" = true ]; then
    echo "=== Building sidecar binary (release) ==="
    cd "$PROJECT_ROOT"
    cargo build --release --target "$TARGET" -p coderouter-proxy
    echo "Sidecar build complete."
fi

if [ "$BUILD_TUI" = true ]; then
    echo "=== Building TUI binary (release) ==="
    cd "$PROJECT_ROOT"
    cargo build --release -p coderouter-tui
    echo "TUI build complete: target/release/${TUI_BIN}"
fi

BUILD_AARCH64=false
if [ "$BUILD_RELEASE" = true ]; then
    if rustup target list --installed 2>/dev/null | grep -q "aarch64-unknown-linux-gnu"; then
        BUILD_AARCH64=true
    else
        echo "Warning: aarch64-unknown-linux-gnu target not installed, skipping aarch64 build."
        echo "  Install with: rustup target add aarch64-unknown-linux-gnu"
    fi
fi

if [ "$BUILD_AARCH64" = true ]; then
    echo "=== Building for aarch64 (cross-compile) ==="
    cd "$PROJECT_ROOT"
    cargo build --release -p coderouter-proxy --target aarch64-unknown-linux-gnu
    cargo build --release -p coderouter-tui --target aarch64-unknown-linux-gnu
    echo "aarch64 build complete."
fi

if [ "$BUILD_GUI" = true ]; then
    echo "=== Copying sidecar to tauri binaries ==="
    mkdir -p "$BINARIES_DIR"
    cp "$PROJECT_ROOT/target/$TARGET/release/$SIDECAR_BIN" "$BINARIES_DIR/$SIDECAR_TRIPLE"
    chmod +x "$BINARIES_DIR/$SIDECAR_TRIPLE"
    echo "Sidecar binary copied to $BINARIES_DIR/$SIDECAR_TRIPLE"

    echo "=== Running tauri build ==="
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
    echo "=== Signing AppImage ==="
    APPIMAGE="$(find "$PROJECT_ROOT/target/release/bundle/appimage/" -name "*.AppImage" 2>/dev/null | head -1)"
    if [ -n "$APPIMAGE" ] && [ -f "$SIGN_KEY" ]; then
      echo "Signing $APPIMAGE..."
      cargo tauri signer sign -f "$SIGN_KEY" -p "" "$APPIMAGE"
      echo "Signing complete."
    elif [ -z "$APPIMAGE" ]; then
      echo "Warning: No AppImage found to sign."
    else
      echo "Warning: No update.key found — skipping signing. Generate one with: cargo tauri signer generate -w ./update.key -p ''"
    fi

    echo ""
    echo "=== GUI Build complete ==="
    echo "AppImage output:"
    find "$PROJECT_ROOT/target/release/bundle/appimage/" -name "*.AppImage" 2>/dev/null || echo "No AppImage found — check build logs above."
fi

if [ "$BUILD_RELEASE" = true ]; then
    echo ""
    echo "=== Packaging release artifacts ==="

    DIST_DIR="$PROJECT_ROOT/dist"
    mkdir -p "$DIST_DIR"

    APPIMAGE="$(find "$PROJECT_ROOT/target/release/bundle/appimage/" -name "*.AppImage" 2>/dev/null | head -1)"
    if [ -n "$APPIMAGE" ]; then
        cp "$APPIMAGE" "$DIST_DIR/CodeRouter_${VERSION}_amd64.AppImage"
        echo "Copied AppImage → dist/CodeRouter_${VERSION}_amd64.AppImage"
        if [ -f "${APPIMAGE}.sig" ]; then
            cp "${APPIMAGE}.sig" "$DIST_DIR/CodeRouter_${VERSION}_amd64.AppImage.sig"
            echo "Copied signature → dist/CodeRouter_${VERSION}_amd64.AppImage.sig"
        fi
    fi

    STAGE="$DIST_DIR/.stage-x86"
    STAGEDIR="$STAGE/coderouter-tui-${VERSION}-linux-x86_64"
    mkdir -p "$STAGEDIR"
    cp "$PROJECT_ROOT/target/release/coderouter-tui" "$STAGEDIR/"
    cp "$PROJECT_ROOT/target/release/coderouter-proxy" "$STAGEDIR/"
    chmod +x "$STAGEDIR/coderouter-tui" "$STAGEDIR/coderouter-proxy"
    (cd "$STAGE" && tar czf "$DIST_DIR/coderouter-tui-${VERSION}-linux-x86_64.tar.gz" "coderouter-tui-${VERSION}-linux-x86_64")
    rm -rf "$STAGE"
    echo "Created dist/coderouter-tui-${VERSION}-linux-x86_64.tar.gz"

    if [ "$BUILD_AARCH64" = true ]; then
        STAGE="$DIST_DIR/.stage-arm"
        STAGEDIR="$STAGE/coderouter-tui-${VERSION}-linux-aarch64"
        mkdir -p "$STAGEDIR"
        cp "$PROJECT_ROOT/target/aarch64-unknown-linux-gnu/release/coderouter-tui" "$STAGEDIR/"
        cp "$PROJECT_ROOT/target/aarch64-unknown-linux-gnu/release/coderouter-proxy" "$STAGEDIR/"
        chmod +x "$STAGEDIR/coderouter-tui" "$STAGEDIR/coderouter-proxy"
        (cd "$STAGE" && tar czf "$DIST_DIR/coderouter-tui-${VERSION}-linux-aarch64.tar.gz" "coderouter-tui-${VERSION}-linux-aarch64")
        rm -rf "$STAGE"
        echo "Created dist/coderouter-tui-${VERSION}-linux-aarch64.tar.gz"
    fi

    SIGNATURE=""
    if [ -f "$DIST_DIR/CodeRouter_${VERSION}_amd64.AppImage.sig" ]; then
        SIGNATURE=$(tr -d '\n' < "$DIST_DIR/CodeRouter_${VERSION}_amd64.AppImage.sig")
    fi

    cat > "$DIST_DIR/latest.json" <<EOF
{
  "version": "${VERSION}",
  "notes": "See https://github.com/CWinthorpe/codeRouter/releases/tag/v${VERSION} for details",
  "pub_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "platforms": {
    "linux-x86_64": {
      "signature": "${SIGNATURE}",
      "url": "https://github.com/CWinthorpe/codeRouter/releases/download/v${VERSION}/CodeRouter_${VERSION}_amd64.AppImage"
    },
    "linux-aarch64": {
      "signature": "",
      "url": "https://github.com/CWinthorpe/codeRouter/releases/download/v${VERSION}/coderouter-tui-${VERSION}-linux-aarch64.tar.gz"
    }
  }
}
EOF
    echo "Created dist/latest.json"
    echo ""
    echo "=== Release artifacts ==="
    ls -lh "$DIST_DIR"/CodeRouter_* "$DIST_DIR"/coderouter-tui-* "$DIST_DIR"/latest.json 2>/dev/null
fi

if [ "$BUILD_TUI" = true ] && [ "$BUILD_RELEASE" = false ]; then
    echo ""
    echo "=== TUI Build output ==="
    echo "Binary: $PROJECT_ROOT/target/release/${TUI_BIN}"
fi

echo ""
echo "=== Build finished ==="
