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

dev:
	cargo run -p xtask -- dev

ports:
	cargo run -p xtask -- ports

[working-directory: 'web']
pages-build:
	env -u NO_COLOR trunk build --release --dist ../dist/pages --no-default-features --features web-static

worker-build-assets:
	cargo run -p xtask -- stage-assets --release

worker-build:
	cargo run -p xtask -- worker-build

worker-dev:
	cargo run -p xtask -- worker

worker-deploy:
	cargo run -p xtask -- worker-deploy

deploy:
	WORKER_ROUTE_HOST="${WORKER_ROUTE_HOST:-sugoijan.dev}" cargo run -p xtask -- worker-deploy

caddy:
	cargo run -p xtask -- caddy

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

web:
	cargo run -p xtask -- web
