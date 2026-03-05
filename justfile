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

[working-directory: 'web/vendor']
gen-font-iosevka:
	@cp Iosevka-custom/private-build-plans.toml Iosevka/
	@cd Iosevka && npm install && npm run build -- woff2-unhinted::IosevkaCustom

[working-directory: 'web']
web:
	trunk serve
