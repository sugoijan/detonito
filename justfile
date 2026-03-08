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
	CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo nextest run -p detonito-core --target wasm32-unknown-unknown --test wasm_smoke

bench *args:
	cargo nextest bench {{args}}

clean: clean-bench

clean-bench:
	rm -rf target/criterion
	cargo clean -p detonito-core

sync-openmoji OPENMOJI_DIR='../openmoji':
	cargo run -p xtask -- sync-openmoji --openmoji-dir "{{OPENMOJI_DIR}}"

regen-fonts:
	cargo run -p xtask -- regen-fonts

regen-sprite:
	cargo run -p xtask -- regen-sprite

regen-assets OPENMOJI_DIR='../openmoji':
	cargo run -p xtask -- sync-openmoji --openmoji-dir "{{OPENMOJI_DIR}}"
	cargo run -p xtask -- regen-fonts

[working-directory: 'web']
web:
	trunk serve --cargo-profile web-dev
