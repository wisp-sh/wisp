# Use a C++ package from native Go or Wasm

The same [Go application](go/main.go) imports a packaged C++ function in both
builds:

```go
package main

import (
    "fmt"

    arithmetic "example.org/cpp-arithmetic"
)

func DoubledSum(a, b uint32) uint32 {
    return arithmetic.Add(a, b) * 2
}

func main() {
    fmt.Println(DoubledSum(20, 1))
}
```

The [C++ implementation](cpp/arithmetic.cpp) is ordinary code too:

```cpp
unsigned add(unsigned a, unsigned b) {
    return a + b;
}
```

Wisp generates the contracts, bindings, and runtime selection. Application
code imports no Wasm or runtime API. For the reverse direction, see
[C++ importing a Go package](../cpp-import-go/README.md).

## Run natively

From the repository root, with Go 1.27 installed:

```sh
examples/go-import-wasm/run.sh --native
# 42
```

On first use, the script runs [install.sh](install.sh). It installs the example
package and its dependencies with ordinary Go module tooling. Native builds
link Wago automatically and embed the C++ Component. They work with cgo disabled
and need no Wasmtime executable, Wisp CLI, Rust compiler, or C++ compiler.

After installation, normal Go commands work directly:

```sh
cd examples/go-import-wasm/go
go run .
# 42
mkdir -p ../../../out
go build -o ../../../out/go-import-native .
../../../out/go-import-native
# 42
```

The [local module proxy](proxy/example.org/cpp-arithmetic/@v) contains the
published `v0.3.0` fixture because `example.org` is an example domain.
`install.sh` uses that proxy for the fixture and the configured Go proxy for
other dependencies. [go.mod](go/go.mod) pins the versions and
[go.sum](go/go.sum) records their checksums. There is no `replace` directive.
Upstream native runtime binaries are downloaded during installation and ignored
by Git. The generated Wisp module is retained in [vendor](go/vendor) so the
Wasm example can also build from a clean checkout without network access.

## Build the same application as Wasm

[wisp.toml](wisp.toml) selects the Go compiler and the same Go directory:

```toml
[package]
name = "demo:go-import-wasm"
version = "0.1.0"

[build]
compiler = "go"
project = "go"
```

With [the compiler tools configured](../../docs/project-builds.md#configure-the-compilers)
and `WISP_WASMTIME` set, run from the repository root:

```sh
cargo build --locked -p wisp
examples/go-import-wasm/run.sh out/my-go-wasm
# 42
```

This compiles the unchanged Go source with the Go toolchain. Wisp discovers the
dependency through its installed module, selects Wasm imports instead of the
native runtime, and composes the C++ implementation into the result. The build
needs the Go toolchain, Wasmtime, the Preview 1 reactor adapter, and WAC. It uses
the checked-in package sources, so no C++ compiler, native Go compiler, or
registry is needed.

The script calls the exported `DoubledSum` function in
`out/my-go-wasm/component.wasm`. Its call to C++ addition returns the same
`42` as the native executable. Choose a fresh output directory for each build.

## Build and publish the C++ package

[The producer config](cpp/wisp.toml) selects the C++ compiler and its source:

```toml
[build]
compiler = "clang"
sources = ["arithmetic.cpp"]
```

Build the release and its Go module:

```sh
target/debug/wisp build examples/go-import-wasm/cpp \
  --go-module example.org/cpp-arithmetic \
  --out out/cpp-package
```

Wisp derives the interface from `add` and emits Cargo, npm, Component, and Go
proxy artifacts. One Go module includes both native and Wasm implementations.
Publish with `wisp library prepare` and `wisp library publish` as described in
[library publication](../../docs/library-publication.md).

For an externally hosted module, use normal Go installation commands with its
proxy configured:

```sh
cd examples/go-import-wasm/go
go mod download
GOOS=wasip1 GOARCH=wasm CGO_ENABLED=0 go mod vendor
```

That vendor directory serves both native Go and Wasm builds.

## Run the registry demonstration

```sh
python3 examples/go-import-wasm/registry-demo.py \
  --release out/cpp-package --out out/registry-demo
```

The demo publishes the package locally and creates a consumer outside the
checkout. Go downloads into an empty module cache and vendors the dependencies.
The demo removes the proxy and module cache, then runs `go run` and compiles
the exact same application source with the Go toolchain. Both native execution and the
composed Wasm export must return `42`. A modified binding must be rejected.
Set `WISP_GO` to override the Go executable. The consumer has no C++ source,
authored WIT, export annotations, or application binding files.

## Inspect the generated files

These files are generated package internals; application authors do not edit
them:

- [bindings_native.go](go/vendor/example.org/cpp-arithmetic/bindings_native.go)
  embeds the Component and calls the bundled Wasmtime runtime.
- [bindings_wasip1.go](go/vendor/example.org/cpp-arithmetic/bindings_wasip1.go)
  exposes the same `Add` function through a typed Wasm import.
- [component.wasm](go/vendor/example.org/cpp-arithmetic/component.wasm),
  [component.wit](go/vendor/example.org/cpp-arithmetic/component.wit), and
  [wisp-component.json](go/vendor/example.org/cpp-arithmetic/wisp-component.json)
  carry the implementation, contract, identity, version, and hash.
- [The application export wrapper](bindings/go/wisp_bindings.go) exposes
  `DoubledSum` to the Component host.
- [The C++ wrapper](bindings/cpp/unit.cpp) and [header](bindings/cpp/wisp.h)
  connect the inferred C++ API to the Component contract.
- [composition.wac](bindings/composition.wac) connects the Go imports to the
  C++ package. [component.wit](bindings/component.wit) records the final API
  and WASI imports.

[compile_flags.txt](compile_flags.txt) configures clangd for the C++ snapshots.
The wrapper's source include is relative to this checkout; the actual compiler
input uses its sandbox path. Each real build retains its staged source,
contracts, bindings, commands, and composition report under `build/`.

The source-inference profile supports scalar signatures, including Go `int`,
`uint`, and local scalar types and aliases. Private Go functions, C++ `static`
functions, and anonymous namespaces remain internal. Unsupported public
signatures receive source-level errors.
