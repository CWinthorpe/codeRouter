#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")" && pwd)"
TAURI_DIR="$PROJECT_ROOT/src-tauri"
VERSION=$(grep '"version"' "$TAURI_DIR/tauri.conf.json" | head -1 | sed 's/.*: *"\([^"]*\)".*/\1/')
REPO="CWinthorpe/codeRouter"
TAG="v${VERSION}"

echo "=== Building release artifacts for ${TAG} ==="
"$PROJECT_ROOT/build.sh" --release

echo ""
echo "=== Preparing release notes ==="
PREV_TAG=$(git describe --tags --abbrev=0 2>/dev/null || echo "")
if [ -n "$PREV_TAG" ]; then
  NOTES=$(git log --oneline "${PREV_TAG}..HEAD" | sed 's/^/- /')
else
  NOTES="Initial release"
fi
echo "$NOTES"

echo ""
echo "=== Creating GitHub release ${TAG} ==="
if gh release view "$TAG" --repo "$REPO" &>/dev/null; then
  echo "Release ${TAG} already exists — uploading artifacts."
  gh release upload --clobber "$TAG" --repo "$REPO" \
    "$PROJECT_ROOT/dist/CodeRouter_${VERSION}_amd64.AppImage" \
    "$PROJECT_ROOT/dist/CodeRouter_${VERSION}_amd64.AppImage.sig" \
    "$PROJECT_ROOT/dist/coderouter-tui-${VERSION}-linux-x86_64.tar.gz" \
    "$PROJECT_ROOT/dist/latest.json"

  if [ -f "$PROJECT_ROOT/dist/coderouter-tui-${VERSION}-linux-aarch64.tar.gz" ]; then
    gh release upload --clobber "$TAG" --repo "$REPO" \
      "$PROJECT_ROOT/dist/coderouter-tui-${VERSION}-linux-aarch64.tar.gz"
  fi
else
  ARGS=(
    "$TAG"
    --repo "$REPO"
    --title "v${VERSION}"
    --notes "$NOTES"
    "$PROJECT_ROOT/dist/CodeRouter_${VERSION}_amd64.AppImage"
    "$PROJECT_ROOT/dist/CodeRouter_${VERSION}_amd64.AppImage.sig"
    "$PROJECT_ROOT/dist/coderouter-tui-${VERSION}-linux-x86_64.tar.gz"
    "$PROJECT_ROOT/dist/latest.json"
  )

  if [ -f "$PROJECT_ROOT/dist/coderouter-tui-${VERSION}-linux-aarch64.tar.gz" ]; then
    ARGS+=("$PROJECT_ROOT/dist/coderouter-tui-${VERSION}-linux-aarch64.tar.gz")
  fi

  gh release create "${ARGS[@]}"
fi

echo ""
echo "=== Release ${TAG} published ==="
echo "https://github.com/$REPO/releases/tag/$TAG"
