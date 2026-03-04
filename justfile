set positional-arguments

default: help

help:
	@just --list

check:
	cargo check

test: test-local test-wasm

test-local:
	cargo nextest run

test-wasm:
	CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test -p detonito-core --target wasm32-unknown-unknown --test wasm_smoke

bench *args:
	cargo bench {{args}}

clean-bench:
	rm -rf target/criterion
	cargo clean -p detonito-core
