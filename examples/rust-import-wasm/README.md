# Import Go and C++ packages from ordinary Rust

The [Rust application](rust/src/lib.rs) uses normal Cargo dependencies:

```rust
pub fn doubled_sum(a: u32, b: u32) -> u32 {
    go_arithmetic::add(a, b) + cpp_arithmetic::add(a, b)
}
```

Both imported implementations compute `a + b`; calling this export with
`20, 1` returns `42`. Additional functions pass a boolean through Go and a
64-bit integer through C++.

[Cargo.toml](rust/Cargo.toml) uses ordinary dependency aliases and pinned versions:

```toml
[dependencies.go_arithmetic]
package = "wisp-demo--go-arithmetic"
version = "=0.1.0"
path = "deps/go_arithmetic"

[dependencies.cpp_arithmetic]
package = "wisp-demo--cpp-math"
version = "=0.1.0"
path = "deps/cpp_arithmetic"
```

The installed crates contain generated Rust APIs, Component metadata and the
compiled Go/C++ implementation. Wisp discovers that metadata, validates the
installed bindings, compiles the ordinary Rust calls into Wasm imports and
composes all three implementations. No application WIT, manual export wrapper
or composition graph is required.

## Build the Rust application

With the toolchains configured:

```sh
source .tools/build/env.sh
examples/rust-import-wasm/run.sh out/my-rust-import-wasm
# 42
```

This runs the packaged Rust compiler inside Wasm. Go and C++ compilers are
unnecessary because their Components are already installed. Cargo is used for
metadata and feature resolution, and Rust compilation uses the pinned Wasm
compiler. The result retains the imported Go runtime's WASI requirements.

The Rust dependency sources also expose the same signatures to host editors,
so this ordinary type check works without compiling the application to Wasm:

```sh
cargo check --offline --locked \
  --manifest-path examples/rust-import-wasm/rust/Cargo.toml
```

Host API stubs deliberately panic if executed; the executable implementation
is selected for the Wasm application. The generated dependencies are available
for inspection in [rust/deps](rust/deps), including their private Wasm imports
and `wisp-component.json` descriptors.

## Install published packages

Start with an admitted release installed by `wisp library fetch`, then ask
Wisp to materialize its Rust guest crate:

```sh
target/debug/wisp library rust-guest out/installed-go-package \
  --out examples/rust-import-wasm/rust/deps/go_arithmetic
```

The crate name comes from the package identity. Cargo aliases let application
code use the desired local name. Use a fresh output directory when regenerating
a crate. Its source and descriptor are generated internals, validated against
the embedded Component on every build.

These `wasm32-wasip1` guest crates also support strings, byte buffers, records
and synchronous typed results. They use the ordinary Rust `String`, `Vec<u8>`
and `Result<T, E>` representations. Resources, asynchronous values and other
list elements are outside this source-compiler projection. The
[rich import verifier](../../tests/source-build/values-interop.py) exercises an
installed Go provider through both Rust and C++ consumers.

## Verify publication and imports

Build the producer releases with their normal configs:

```sh
target/debug/wisp build examples/build-rust \
  --go-module example.org/rust-arithmetic --out out/rust-math-release
target/debug/wisp build examples/cpp-import-go/go --out out/go-arithmetic-release
target/debug/wisp build examples/build-cpp --out out/cpp-math-release

python3 examples/rust-import-wasm/verify.py \
  --rust-release out/rust-math-release \
  --go-release out/go-arithmetic-release \
  --cpp-release out/cpp-math-release \
  --clangd /path/to/clangd \
  --out out/rust-interop-verification
```

The script publishes all three releases to disposable local registries,
installs them through Wisp and Go module tooling, and stops the registries.
It then checks these four flows using real compilers and execution:

- Go imports Rust.
- C++ imports Rust.
- Rust imports Go.
- Rust imports C++.

Every consumer returns `42`; boolean and high 64-bit values are also checked.
The Go proxy and module cache are removed before its build. Go/C++ consumers
have invalid Rust compiler paths, and the Rust consumer has invalid Go/C++
compiler paths. Cargo's compiler queries and Rust source compilation use the
pinned Wasm compiler. A native Rust guard rejects source compilation. Modified
generated Rust bindings must be rejected without creating a build output.

The same check runs clangd against the generated C++ header in the installed
example and a relocated copy, then verifies that an invalid argument fails.
Cargo checks the installed Rust dependencies, and the native Go run checks
the Go package's ordinary host implementation.

Each verification run uses a fresh local registry. When a newly compiled Rust
Component changes the fixed `v0.1.0` fixture archive, the verifier refreshes only
that module/version's checksum entries in its copied `go.sum`. Other dependency
hashes and the checked-in example stay pinned. `go-fixture-sums.json` records the
removed fixture hashes and the newly installed sums. The ordinary
`go-import-rust/install.sh` keeps its normal checksum checks. The verifier's
checksum-database exemption covers the local fixture; public dependencies keep
their normal Go settings.

## Recheck with the source-built compiler

The local environment selects the verified source-built compiler through
`../wisp-buildtools/rust/dist/compiler`. Build a new Rust producer before running
the registry verifier:

```sh
source .tools/build/env.sh
target/debug/wisp build examples/build-rust \
  --go-module example.org/rust-arithmetic \
  --cache .tools/build/runtime-cache \
  --out out/rust-producer-macro-release

python3 -I examples/rust-import-wasm/verify.py \
  --cli target/debug/wisp \
  --rust-release out/rust-producer-macro-release \
  --go-release out/cpp-import-go-producer \
  --cpp-release out/rust-interop-cpp-producer \
  --clangd /path/to/clangd \
  --out out/rust-interop-macro-verification
```

These Go/C++ release paths reuse the already admitted arithmetic producers.
The verifier installs new consumer copies and leaves the checked-in generated
providers unchanged. Use fresh output directories for each rerun. Standalone
Serde/compiler acceptance and the Wisp Serde check remain separate from these
four arithmetic import flows.

Run the [Wisp Serde check](../../tests/source-build/rust-serde.md) separately
when changing macro or build-script support. The compiler repository's
[acceptance suite](https://github.com/wisp-sh/wisp-buildtools/tree/main/rust/tests)
covers the standalone compiler cases.
