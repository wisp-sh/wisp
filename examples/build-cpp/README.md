# Build ordinary C++ as a package

[math.cpp](math.cpp) defines ordinary free functions. Wisp infers their public
signatures and generates the Wasm contract and bindings internally. `static`
functions and anonymous namespaces stay internal.

```cpp
unsigned add(unsigned a, unsigned b) { return a + b; }
```

[wisp.toml](wisp.toml) selects the C++ compiler and the source files:

```toml
[build]
compiler = "clang"
sources = ["math.cpp"]
```

With [the tools configured](../../docs/project-builds.md#configure-the-compilers),
run from the repository root:

```sh
target/debug/wisp build examples/build-cpp --component-only --out out/my-cpp
"$WISP_WASMTIME" run --invoke 'add(20, 22)' out/my-cpp/component.wasm
# 42
```

Choose a fresh output directory. Omit `--component-only` to generate the Cargo
and npm packages too. Add `--go-module example.org/my-cpp` to emit a Go
module whose imports work in native Go and Wasm builds. Native Go links
Wago automatically; application code uses ordinary functions. See
[the Go import example](../go-import-wasm/README.md) for both run commands.
Generated contracts and glue are retained under
`build/units/application/` in the output for inspection.

Inference supports scalar free functions, plain records, `enum class`,
`std::string`, `std::vector<T>`, `std::optional<T>` and `std::expected<T,E>`.
The [rich-value example](../build-values/cpp/main.cpp) also uses the compatible
`wisp::string`, `wisp::bytes` and `wisp::result` source helpers. Pointers,
references, overloaded public names, public templates and conditional API
definitions are rejected.

Wisp adapts these source values to upstream wit-bindgen C++ values. Upstream
bindings own the Component ABI and return cleanup. The Wasm-hosted compiler
uses WASI SDK libc/libc++; configure `WISP_CLANG_WASI_SYSROOT` and
`WISP_WIT_BINDGEN` as described in the compiler setup. There is no handwritten
WIT or binding header.
