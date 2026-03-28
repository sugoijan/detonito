#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
WORKER_DIR=$(cd -- "$SCRIPT_DIR/.." && pwd)
REPO_ROOT=$(cd -- "$WORKER_DIR/.." && pwd)

export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"

install_rustup() {
  curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal --default-toolchain stable
}

ensure_cargo_tool() {
  local binary="$1"
  local version="$2"

  if command -v "$binary" >/dev/null 2>&1; then
    local output
    output=$("$binary" --version 2>/dev/null || true)
    if [[ "$output" == *"$version"* ]]; then
      return
    fi
  fi

  cargo install --locked --version "$version" "$binary"
}

if ! command -v rustup >/dev/null 2>&1; then
  install_rustup
fi

# shellcheck disable=SC1090
source "$CARGO_HOME/env"

rustup toolchain install stable --profile minimal
rustup default stable
rustup target add wasm32-unknown-unknown

ensure_cargo_tool trunk 0.21.14
ensure_cargo_tool worker-build 0.7.5

cd "$REPO_ROOT"
cargo run -p xtask -- worker-prepare-deploy
