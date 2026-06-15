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

## Rules

- `rustfmt.toml` uses unstable rustfmt options; run formatting through nightly Cargo.
- Preserve `SetDns` close/drop restoration semantics when changing platform backends.
- Linux split DNS requires `Config::device` and systemd-resolved; Windows ignores `device`; macOS ignores `device` for split DNS.
