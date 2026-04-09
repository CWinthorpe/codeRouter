# Release v0.1.7: Version Bump, Build, and Release

## Task

Bump version from 0.1.6 to 0.1.7 across all config files, build the AppImage, and create a GitHub release.

## Step 1: Version Bump

Update version from `0.1.6` to `0.1.7` in these files:

1. `sidecar/Cargo.toml` — line 3: `version = "0.1.7"`
2. `src-tauri/Cargo.toml` — line 3: `version = "0.1.7"`
3. `src-tauri/tauri.conf.json` — line 4: `"version": "0.1.7"`
4. `package.json` — line 4: currently `"0.1.5"`, update to `"0.1.7"`

After editing, run `cargo check --workspace` to verify Cargo.lock gets updated.

## Step 2: Build AppImage

Run the build script:
```bash
bash build.sh
```

This will:
1. Build the sidecar binary in release mode
2. Copy it to src-tauri/sidecar/
3. Run `npm run tauri build` to produce the AppImage

The output AppImage will be in `target/release/bundle/appimage/`.

## Step 3: Create GitHub Release

Create a release with tag `v0.1.7` using `gh release create`:

```bash
gh release create v0.1.7 \
  target/release/bundle/appimage/*.AppImage \
  --title "CodeRouter v0.1.7" \
  --notes "## Changes since v0.1.6

- Fix streaming connections dropping at 30 seconds (TTFB-only timeout instead of total request timeout)
- Handle OpenRouter string pricing values and top_provider metadata
- Parse model metadata (context window, pricing) from list responses for Venice and other providers
- Wire pricing from ProviderModel to RequestEvent for accurate cost tracking
- Remove client total timeout that killed long-running streams

## Download

- \`CodeRouter_0.1.7_amd64.AppImage\`"
```

## Step 4: Git Commit

Commit the version bump changes (Cargo.toml, package.json, tauri.conf.json, Cargo.lock, sidecar binary) with message:
```
chore: bump version to 0.1.7
```

Push to origin.

## Acceptance Criteria

- All 4 files have version 0.1.7
- AppImage builds successfully
- GitHub release v0.1.7 is created with the AppImage asset
- Git commit pushed to main
