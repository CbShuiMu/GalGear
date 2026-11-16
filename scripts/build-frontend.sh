#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
profile=debug
flags=()
if [[ "${1:-}" == "--release" ]]; then profile=release; flags+=(--release); fi
version=$(sed -n 's/^wasm-bindgen = "=\([^"]*\)".*/\1/p' crates/ui/Cargo.toml)
if [[ "$(wasm-bindgen --version)" != "wasm-bindgen $version" ]]; then
  echo "Install the matching CLI: cargo install wasm-bindgen-cli --version $version --locked" >&2
  exit 1
fi
cargo build -p galgear-ui --target wasm32-unknown-unknown --locked "${flags[@]}"
mkdir -p dist
wasm-bindgen --target web --out-dir dist/pkg --out-name galgear_ui "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/$profile/galgear_ui.wasm"
cp web/*.html web/*.css web/*.js dist/
cp -R web/assets dist/
