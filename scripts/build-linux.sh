#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
bash scripts/build-frontend.sh --release
# tauri.linux.conf.json is merged automatically by Tauri on Linux.
cargo build -p galgear --release --locked
echo "Built ${CARGO_TARGET_DIR:-target}/release/galgear"
