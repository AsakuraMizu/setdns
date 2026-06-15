# AGENTS.md

## Project

- Rust library for temporarily applying and restoring global or split system DNS configuration.

## Commands

| Task         | Command                                     |
| ------------ | ------------------------------------------- |
| Test         | `cargo test`                                |
| Format check | `cargo +nightly fmt --all -- --check`       |
| Clippy       | `cargo clippy --all-targets --all-features` |

## Structure

- `src/config.rs`: validates public `Config` into normalized owner, server, domain, and device state before platform code runs.
- `src/platform/`: OS-specific DNS backends selected by `cfg(target_os)`.

## Safety rules

- `test-runner/` contains a destructive integration test executable.
- Agents MUST NOT run `test-runner`, `cargo run --manifest-path test-runner/Cargo.toml`, or any wrapper that executes it unless the user explicitly requests that exact execution.
- Agents MAY run non-executing checks such as `cargo check --manifest-path test-runner/Cargo.toml` and format checks.

## Rules

- `rustfmt.toml` uses unstable rustfmt options; run formatting through nightly Cargo.
- Preserve `SetDns` close/drop restoration semantics when changing platform backends.
- Linux split DNS requires `Config::device` and systemd-resolved; Windows ignores `device`; macOS ignores `device` for split DNS.
