# Build ordinary Rust with a compiler running in Wasm

The [Rust library](rust/src/lib.rs) exports ordinary public functions. Its
private helper uses a generic iterator, `Result`, `String` and `Vec` internally:

```rust
pub fn add(a: u32, b: u32) -> u32 {
    let values = vec![a, b];
    sum(values).expect("the arithmetic sum is available")
}
```

Wisp infers the scalar public API and generates the Component contract and ABI
wrappers. There is no authored WIT, export annotation or special function name.
The source remains a normal Cargo library and can be checked in a Rust editor.

With the compiler toolchains configured:

```sh
source .tools/build/env.sh
target/debug/wisp build examples/build-rust --component-only --out out/rust-math
"$WISP_WASMTIME" run --invoke 'add(20,22)' out/rust-math/component.wasm
# 42
```

`WISP_RUST` selects the compiler package. The local environment defaults to
`../wisp-buildtools/rust/dist/compiler`, the verified source-built compiler.
The compiler requires Wasmtime 50.0.0, selected with `WISP_RUST_WASMTIME`.
Wisp uses its normal Wasmtime runtime for the resulting Component. Cargo reads
the dependency graph; the Wasm-hosted Rust compiler compiles the Rust code.

To produce an admitted release and a normal Go module as well:

```sh
target/debug/wisp build examples/build-rust \
  --go-module example.org/rust-arithmetic --out out/rust-math-release
```

See [Go importing Rust](../go-import-rust),
[C++ importing Rust](../cpp-import-rust), and
[Rust importing Go and C++](../rust-import-wasm) for package imports in both
directions. This example has a scalar interface. Source APIs can also expose
`String`, `Vec<u8>`, public named structs and `Result<T, E>`; the
[rich value example](../build-values/rust/) exercises those types. Generic
functions remain private implementation details.

This arithmetic example checks ordinary Rust exports. The source-built compiler
also passes unmodified Serde derives and basic build scripts in the separate
[compiler acceptance suite](https://github.com/wisp-sh/wisp-buildtools/blob/main/rust/tests/README.md)
and [Wisp Serde check](../../tests/source-build/rust-serde.md).
