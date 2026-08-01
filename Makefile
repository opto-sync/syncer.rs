.PHONY: test test-wasm native wasm pkg-web test-browser test-all clean

# The wasm module is feature-gated, so the default target does not compile it.
test:
	cargo test --all-targets
	cargo test --all-targets --features wasm

native:
	cargo build --release

# Bundler output, for consumers.
wasm:
	wasm-pack build --release --target bundler -- --features wasm

# Web output, for the conformance suites. `--target web` is what both the Node
# runner and the browser spec load, so they exercise identical artifacts.
pkg-web:
	wasm-pack build --release --target web --out-dir pkg-web -- --features wasm

# Executes the shared corpus under Node. Fast; no browser download.
test-wasm: pkg-web
	node tests/wasm/run-node.mjs

# Executes the same corpus in real Chromium.
test-browser: pkg-web
	cd tests/wasm && npm ci && npx playwright install chromium && npm run test:browser

test-all: test test-wasm test-browser

clean:
	cargo clean
	rm -rf pkg pkg-web tests/wasm/node_modules tests/wasm/playwright-report tests/wasm/test-results
