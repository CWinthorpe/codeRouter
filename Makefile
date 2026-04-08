.PHONY: build dev test clean

build:
	@bash build.sh

dev:
	@PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig npm run tauri dev

test:
	@cargo test --workspace

clean:
	cargo clean
	rm -rf dist/
	rm -rf node_modules/.vite/
