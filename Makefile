.PHONY: test native wasm clean

test:
	cargo test --all-targets

native:
	cargo build --release

wasm:
	wasm-pack build --release --target bundler -- --features wasm

clean:
	cargo clean
	rm -rf pkg
