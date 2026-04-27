.PHONY: build dev test clean tui tui-dev tui-arm64

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
	rm -rf src-tauri/gen/
	rm -rf src-tauri/target/
	rm -rf target/

tui:
	cargo build --release -p coderouter-tui
	@echo "TUI binary: target/release/coderouter-tui"

tui-dev:
	cargo run -p coderouter-tui

tui-arm64:
	cargo build --release -p coderouter-tui --target aarch64-unknown-linux-gnu
	@echo "ARM64 TUI binary: target/aarch64-unknown-linux-gnu/release/coderouter-tui"
