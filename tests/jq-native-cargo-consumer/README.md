# jq installed-Cargo consumer

Run after packing the unchanged `fixtures/jq/component.wasm` with this worktree's Wisp executable:

```sh
wisp library pack fixtures/jq --out /path/to/release
mkdir -p tests/jq-native-cargo-consumer/vendor
tar -xf /path/to/release/rust.crate -C tests/jq-native-cargo-consumer/vendor
CARGO_TARGET_DIR=/path/to/private/executable-target TMPDIR=/path/to/private/executable-tmp \
  cargo test --offline --manifest-path tests/jq-native-cargo-consumer/Cargo.toml --test jq
```

Do not check in extracted vendor contents. `/tmp` may be noexec. The test covers real installed-Cargo jq evaluation, empty output, invalid filter/JSON, wall-clock denial, poison after a host trap, and a fresh client succeeding. The monotonic-clock trap is also installed by the pure linker, but this jq fixture does not expose a filter that invokes it directly.
