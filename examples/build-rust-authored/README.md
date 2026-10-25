# Build Rust with WIT bindings

[wit/world.wit](wit/world.wit) declares the API. The [Rust library](rust/src/lib.rs)
implements the generated `Guest` trait and invokes its `export!` macro.
Wisp stages `src/wisp_bindings.rs` and compiles the root library directly as a
cdylib using the Wasm-hosted Rust compiler. This example selects authored WIT; ordinary Rust APIs can use inference.

```sh
wisp build examples/build-rust-authored --component-only --out out/rust
wasmtime run --invoke 'add(20,22)' out/rust/component.wasm
```

Set `WISP_RUST` to the compiler package and `WISP_WIT_BINDGEN` to the pinned
0.61.1 generator. Cargo resolves locked offline metadata; application compilation
runs in Wasm. See [rich values](../build-values) for nested lists, options, enums,
records and synchronous results. Use an absent output directory.
