# Wisp

**Compile source projects to WebAssembly Components and ship them as native
libraries for other languages.**

> [!WARNING]
> Wisp is pre-alpha. Command-line interfaces, package formats and supported
> targets may change between releases.

Wisp takes a project written in one language, compiles it to a
[WebAssembly Component](https://component-model.bytecodealliance.org/), and
packages that Component so it can be installed from another language:

- a **Rust crate**,
- a **Node package** (server and browser),
- an optional **Go module**, either native or Wasm,
- a dependency for other **Component guests**.

The compilers themselves run as Wasm, so the host does not need a native
toolchain for each source language.

## What it looks like

Write ordinary Go. There are no annotations, bindings or WIT file:

```go
// src/main.go
package main

func Add(a, b uint32) uint32 { return a + b }

func main() {}
```

```toml
# wisp.toml
[package]
name = "demo:go-arithmetic"
version = "0.1.0"

[build]
compiler = "go"
project = "src"
```

Build it once:

```sh
wisp build . --out out/go-arithmetic
```

The release contains a Component, an npm package and a Cargo crate, each with
a typed API inferred from the Go source.

**Node** (`npm install ./out/go-arithmetic/node.tgz`):

```js
import { api } from '@demo/go-arithmetic';

console.log(api.add(20, 22)); // 42
```

**Rust** (`wisp-demo--go-arithmetic = { path = "out/go-arithmetic/rust" }`):

```rust
use wisp_demo__go_arithmetic::Library;

let mut api = Library::new()?.api();
assert_eq!(api.add(20, 22)?, 42);
```

Neither consumer needs Go installed. Node runs the Component through
generated JCO bindings. Native Rust runs it in a bundled Wasmtime. The same
crate also works inside other Wasm Components, and the release can be published
to Cargo, npm, OCI and Go registries. See
[cpp-import-go](examples/cpp-import-go/README.md) for this package consumed
from C++.

## Features

- **Many source languages.** `wisp build` compiles C/C++, Go, Rust, Zig,
  AssemblyScript, Nim, JavaScript, TypeScript, Python, Grain, Bend 2, Goose,
  Swift, Java and Kotlin with Wasm-hosted compilers.
- **Inferred APIs.** Wisp derives contracts from ordinary C++, Go and Rust
  functions, including lists, options, enums, records and synchronous results.
  Pinned upstream `wit-bindgen` handles the ABI. Authored WIT is optional.
- **Cross-language dependencies.** Wisp discovers Components in installed Go
  and Rust packages and generates C++ headers for installed releases. Published
  Go guest modules support rich typed imports.
- **Standard libraries.** Wasm-hosted Clang uses WASI SDK's libc and libc++;
  Go compiles its standard library and upstream binding runtime inside Wasm.
- **Validated releases.** Each build produces a release that can be validated
  and published to local Cargo, npm, OCI or Go registries.

## Installation

Wisp is a Rust program with two executables: the `wisp` CLI and its execution
worker. Build both from source; the first build needs network access.

```sh
git clone https://github.com/wisp-sh/wisp
cd wisp
cargo install --path crates/wisp --locked
cargo install --path crates/wisp-worker --locked
wisp --version
```

Keep `wisp` and `wisp-worker` on your `PATH` or in the same directory. Set
`WISP_WORKER` to use a worker from another location.

To compile source projects you also need the compiler bundles. Install them
with `wisp toolchain install`, set up their shared runtimes with
`wisp toolchain setup-hosts`, and check the result with `wisp toolchain doctor`.
See [toolchain packages](docs/toolchain-packages.md) for details. There is no
automatic public download catalog yet, so each bundle needs an explicit source
or download location.

## Quick start

Inspect the bundled jq Component. This works without compiler bundles:

```sh
wisp inspect fixtures/jq/component.wasm
```

Build a project and validate the release:

```sh
wisp build path/to/project --out out/release
wisp library validate out/release
```

Select the source language with `build.compiler` in the project's TOML
configuration. See [project builds](docs/project-builds.md) for the
configuration reference, tool paths and binding limits.

To package an existing Component, its WIT and package metadata:

```sh
wisp library pack AUTHOR_DIR --out out/library-release \
  --go-module example.org/team/library
wisp library validate out/library-release
```

Omit `--go-module` if you don't need a Go module.

## Examples

| Example | What it shows |
| --- | --- |
| [build-rust](examples/build-rust/README.md) | Build a Rust library |
| [build-go](examples/build-go) | Build a Go library |
| [build-python](examples/build-python/README.md) | Build a Python library with strings, lists, records and typed errors |
| [build-cpp](examples/build-cpp) | Build a C++ library |
| [build-mixed](examples/build-mixed/README.md) | Build C++ and Go together |
| [build-bend](examples/build-bend/README.md) | Build a Bend 2 program |
| [go-import-wasm](examples/go-import-wasm/README.md) | Go importing a published C++ module, natively or as Wasm |
| [go-import-rust](examples/go-import-rust/README.md) | Go importing Rust |
| [cpp-import-go](examples/cpp-import-go/README.md) | C++ importing a Go package |
| [cpp-import-rust](examples/cpp-import-rust/README.md) | C++ importing Rust |
| [rust-import-wasm](examples/rust-import-wasm/README.md) | Rust importing Go and C++ |
| [build-values](examples/build-values/README.md) | Lists, options, enums, records and results in C++, Go and Rust |
| [build-c](examples/build-c/README.md) | C using WASI libc through the Wasm-hosted compiler |

## Platform support

Rust native packages target macOS ARM64 and Linux x86_64. The full list of
consumer platforms, verified features and API limits is in the
[target support matrix](docs/target-support.md) and the
[language support matrix](https://github.com/wisp-sh/wisp-buildtools/blob/main/docs/language-support.md).

## Documentation

Guides:

- [Project builds](docs/project-builds.md): configuration, source languages and binding limits
- [Toolchain packages](docs/toolchain-packages.md): installing compiler bundles
- [Component library packages](docs/component-library-packages.md): package layout and runtimes
- [Go Component libraries](docs/component-library-go.md): native and Wasm Go modules
- [Component dependencies](docs/component-dependencies.md): consuming Components from other packages
- [Library publication](docs/library-publication.md): Cargo, npm, OCI and Go distribution
- [Authored packages](docs/authored-packages.md): WIT and package metadata
- [Target support](docs/target-support.md): consumers and features that work today

[Verification and internals](docs/verification/README.md) covers the
architecture decisions, admission rules, test suites, compiler CI and the
detailed evidence behind the support tables.

## Security and limitations

Wisp is not production-hardened yet. In particular:

- Guest execution has no fuel, deadline, memory or output quotas. Only run
  trusted, bounded Components and inputs.
- Admission checks identity, metadata and byte integrity. It does not prove
  that uploaded wrappers are safe to install.
- Publication has only been exercised against isolated local registries, not
  public ones.
- There is no account or owner verification, source attestation or TLS
  service.

## Contributing

Bug reports and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md)
for development setup and the checks to run before submitting a change.

## License

Wisp is licensed under the [Apache License 2.0](LICENSE). Third-party
notices for bundled components, such as jq and Oniguruma, are kept alongside
those components.
