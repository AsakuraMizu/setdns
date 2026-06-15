#!/usr/bin/env bash
set -euo pipefail

export RUST_LOG=${RUST_LOG:-info}

cargo run --manifest-path test-runner/Cargo.toml -- global
cargo run --manifest-path test-runner/Cargo.toml -- global --tun
