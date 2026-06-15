# Safety rules

This crate contains a destructive integration test executable.

Agents MUST NOT run this binary unless the user explicitly asks for that exact execution.

Forbidden without explicit user request:
- `cargo run --manifest-path test-runner/Cargo.toml -- global`
- `cargo run --manifest-path test-runner/Cargo.toml -- split`
- executing the built `test-runner` binary
- invoking wrappers that execute this binary

Allowed:
- reading source files
- editing source files
- `cargo check --manifest-path test-runner/Cargo.toml`
- format checks

This binary may change system DNS, create TUN devices, bind DNS sockets, and break name resolution.
It is intended for Docker or CI environments.
