# Build ordinary Go as a package

[src/main.go](src/main.go) contains ordinary exported Go functions. Wisp derives
the public API, generates the Wasm export bindings, and compiles with the Go toolchain.
The project config selects the compiler and source directory:

```toml
[build]
compiler = "go"
project = "src"
```

With [the tools configured](../../docs/project-builds.md#configure-the-compilers),
run from the repository root:

```sh
target/debug/wisp build examples/build-go --component-only --out out/my-go
"$WISP_WASMTIME" run --invoke 'add(20, 22)' out/my-go/component.wasm
# 42
```

Choose a fresh output directory. Omit `--component-only` to generate Cargo and
npm packages. Add `--go-module example.org/go-math` to also emit a Go module
that works in native and Wasm builds with the same imports and calls. Native
Go links Wago automatically. See
[the Go import example](../go-import-wasm/README.md) for a consumer using both
targets.

The example also exercises Go clocks, randomness, environment
isolation, filesystem denial, allocation and garbage collection.

Public package-level functions become exports. Private helpers and `main`
remain implementation code. Scalar signatures, `int`, `uint`, and local scalar
types and aliases are supported. Other public signature shapes are rejected
with a source-level error. There are no handwritten WIT files, special export
names, directives, or binding files. Generated contracts and glue remain in the
build output for inspection.
