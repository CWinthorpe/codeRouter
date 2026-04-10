# Release v0.1.8: Version Bump, Build, and Release

## Task

Bump version from 0.1.7 to 0.1.8 across all config files, build the AppImage, and create a GitHub release.

## Step 1: Version Bump

Update version from `0.1.7` to `0.1.8` in these files:

1. `sidecar/Cargo.toml` — line 3: `version = "0.1.8"`
2. `src-tauri/Cargo.toml` — line 3: `version = "0.1.8"`
3. `src-tauri/tauri.conf.json` — line 4: `"version": "0.1.8"`
4. `package.json` — line 4: update to `"0.1.8"`

After editing, run `cargo check --workspace` to verify Cargo.lock gets updated.

## Step 2: Build AppImage

Run the build script:
```bash
bash build.sh
```

The output AppImage will be in `target/release/bundle/appimage/`.

## Step 3: Create GitHub Release

Create a release with tag `v0.1.8`:

```bash
gh release create v0.1.8 \
  target/release/bundle/appimage/*.AppImage \
  --title "CodeRouter v0.1.8" \
  --notes "## Changes since v0.1.7

- Load current agent model assignments on OpenCode Setup tab mount (dropdowns no longer blank out)
- Fix streaming connections dropping at 30 seconds (TTFB-only timeout)
- Handle OpenRouter string pricing values and top_provider metadata
- Parse model metadata (context window, pricing) from list responses for Venice and other providers
- Wire pricing from ProviderModel to RequestEvent for accurate cost tracking

## Download

- \`CodeRouter_0.1.8_amd64.AppImage\`"
```

## Step 4: Git Commit

Commit the version bump changes with message:
```
chore: bump version to 0.1.8
```

Push to origin.

## Acceptance Criteria

- All 4 files have version 0.1.8
- AppImage builds successfully
- GitHub release v0.1.8 is created with the AppImage asset
- Git commit pushed to main
