# Build Go from an authored WIT contract

[wit/world.wit](wit/world.wit) defines the Component API and package identity.
The functions in [src/export_wit_world/implementation.go](src/export_wit_world/implementation.go)
implement that API using the upstream Go backend's export package convention.
The Go module is `example.org/go-math`.

Wisp runs the pinned `wit-bindgen-cli 0.61.1` generator and stages its generated
packages in the build directory. It adds a main-package import of `wit_exports`
to retain the export glue. The application compiler and linker run as Wasm.

The project selects its contract explicitly:

```toml
[build]
compiler = "go"
wit = "wit"
world = "library"
project = "src"
```

With [the tools configured](../../docs/project-builds.md#configure-the-compilers),
run from the repository root:

```sh
target/debug/wisp build examples/build-go-authored --component-only --out out/my-go
"$WISP_WASMTIME" run --invoke 'add(20, 22)' out/my-go/component.wasm
# 42
```

Choose a fresh output directory. Omit `--component-only` to generate Cargo and
npm packages. Add `--go-module example.org/go-math` to emit a Go module for native
and Wasm callers. The example also exercises clocks, randomness, environment
isolation, filesystem denial, allocation, and garbage collection.

The runtime is the pinned `go.bytecodealliance.org/pkg v0.2.3` support bundled
with the Go toolchain. An older toolchain can use a local verified support copy
through `WISP_GO_WIT_RUNTIME`. Application builds do not fetch Go dependencies.
