# Shared type provider regression

`provider.wasm` exports a type-only `demo:a/types@1.0.0` interface and an `api` interface that uses its record. `consumer.wasm` imports both and forwards the typed `echo` call through `demo:b/api@1.0.0`.

These are guest-built Components because hand-written aliases in the reduced WAT fixture triggered a wit-parser 0.245.1 panic before composition. The checked-in bytes let the dependency test run without rebuilding a Rust guest. Rebuild with `python3 crates/wisp/tests/fixtures/shared-type-provider/build.py` after provisioning Rust 1.98.1, its `wasm32-unknown-unknown` target, and the pinned Component tools. The build uses the workspace lock and Cargo cache without network access.
