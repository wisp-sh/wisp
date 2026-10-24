# Import a Rust package from ordinary Go

[main.go](go/main.go) imports a versioned Go module and calls the Rust package:

```go
import arithmetic "example.org/rust-arithmetic"

func DoubledSum(a, b uint32) uint32 {
    return arithmetic.Add(a, b) * 2
}
```

The implementation comes from [the Rust producer](../build-rust/rust/src/lib.rs).
Wisp generates the Go module, bindings and Component composition. The Go source
uses the same import for native execution and a Wasm build.

With the toolchains configured, build the checked-in installed dependency:

```sh
source .tools/build/env.sh
examples/go-import-rust/run.sh out/my-go-import-rust
# 42
```

Only the Go toolchain compiles this application. The installed package already contains
Rust's compiled Component, so this build needs no Rust compiler or Rust source.
The result exports `doubled-sum`, `negate` and `widen` and preserves boolean
and 64-bit values across the import.

For native Go execution:

```sh
examples/go-import-rust/run.sh --native
# 42
```

The first native run uses [install.sh](install.sh) to install the example's
local published module and its runtime dependency with ordinary Go tooling.
It uses Wago and works with `CGO_ENABLED=0`. The local proxy exists because `example.org`
is a fixture domain. There is no `replace` directive in [go.mod](go/go.mod).
The module includes native and Wasm implementations, so editor diagnostics and
completion see an ordinary Go package.

After a fresh checkout, run `examples/go-import-rust/install.sh` once before
opening the Go project in an editor. The example includes its generated Rust
package API and compiled Component. Go tooling installs the native runtime
dependency into the ignored `go/vendor/github.com` directory.

The dependency's generated files live in
[go/vendor/example.org/rust-arithmetic](go/vendor/example.org/rust-arithmetic).
The Wasm binding uses `//go:wasmimport` internally; Wisp discovers its Component
metadata and connects that import to Rust during `wisp build`. The native
binding embeds the same Component and uses the generated runtime adapter.

To rebuild the producer and repeat local publication, module installation,
offline compilation and all four Rust import flows, see
[the verification command](../rust-import-wasm/README.md#verify-publication-and-imports).
No application WIT or ABI annotations are needed.
