# Build C++ and Go together

This advanced example uses explicit WIT contracts and a source-unit graph. For
ordinary application code with inferred contracts, use
[the Go registry example](../go-import-wasm/README.md).

[compile_flags.txt](compile_flags.txt) and the generated
[wisp.h](bindings/cpp/wisp.h) let clangd typecheck the C++ source immediately.
Wisp regenerates the compilation header from the explicit contract during builds.

This example builds C++ with the C++ toolchain, imports its `arithmetic.add` function into
Go, and compiles Go with the Go toolchain. Wisp embeds both in one WebAssembly Component
exporting the Go `calculator.doubled-sum` function.

The call `doubled-sum(20, 1)` adds the arguments in C++, then doubles the result
in Go, returning `42`.

- [wisp.toml](wisp.toml) selects both compilers and wires the dependency.
- [wit/world.wit](wit/world.wit) declares the imported and exported interfaces.
- [math.cpp](math.cpp) implements addition.
- [go/main.go](go/main.go) calls `ImportArithmeticAdd` through generated bindings
  and exports `ExportCalculatorDoubledSum`.
- [go/internal/scale/scale.go](go/internal/scale/scale.go) doubles the sum in a
  local Go package.

For a Go project consuming an already-built `.wasm` file, use
[go-import-wasm](../go-import-wasm/README.md).

## Build and run

Build the Wisp CLI with `cargo build --locked -p wisp` and configure the tools
listed in [project builds](../../docs/project-builds.md#configure-the-compilers).
This example needs the C++ and Go toolchains, Wasmtime, the Preview 1 reactor adapter, and WAC.
Set `WISP_WASMTIME` to the configured Wasmtime executable for the run command.

From the repository root:

```sh
examples/build-mixed/run.sh
# 42
```

The script builds `out/build-mixed-example/component.wasm` and checks that the
Go export returns `42`. Pass an output directory as its first argument to build
again; Wisp requires a fresh directory. `WISP_CLI` overrides the default
`target/debug/wisp`, and `WISP_BUILD_CACHE` overrides the compiler runtime cache.

The equivalent commands are:

```sh
target/debug/wisp build examples/build-mixed --component-only \
  --out out/my-mixed-wasm
"$WISP_WASMTIME" run --invoke 'doubled-sum(20, 1)' \
  out/my-mixed-wasm/component.wasm
# 42
```

The output needs a Component-capable runtime with WASI support. The C++
dependency is embedded, so running the file does not need either compiler.

## Build a publishable package

Omit `--component-only` to generate the Component plus Cargo and npm packages:

```sh
target/debug/wisp build examples/build-mixed --out out/mixed-release \
  --go-module example.org/mixed-calculator
target/debug/wisp library validate out/mixed-release
```

The Go module exposes ordinary scalar calls in native and Wasm applications.
Native builds include Wago automatically. Rust selects its runtime by
target, and the npm package includes its JavaScript and Wasm implementation.

Packaging needs the additional tools and caches described in
[project builds](../../docs/project-builds.md#configure-the-compilers).
Follow [library publication](../../docs/library-publication.md) to prepare and
publish the release.
