.PHONY: build dev test

build:
	@bash build.sh

dev:
	@PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig npm run tauri dev

test:
	@cargo test --workspace
