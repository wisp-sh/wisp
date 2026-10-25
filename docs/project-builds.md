# Build a project from source

`wisp build` reads a project's `wisp.toml`, runs the selected Wasm-hosted
compiler, and packages its Component through the schema-3 library packer and
admission checks. It supports C/C++, Go, Rust, Zig, AssemblyScript, Nim,
JavaScript, TypeScript, Python, Grain, Bend 2, Goose, Swift, Java and Kotlin.

```sh
wisp build path/to/project --out out/release
wisp library validate out/release
wisp library prepare out/release --config publisher.json --out publication.json
wisp library publish publication.json --state publication-state.json
```

The build never publishes automatically. The publication configuration and
transports are described in [library-publication.md](library-publication.md).

With no project argument, Wisp reads `./wisp.toml`. A TOML file path can also be
passed directly. With no `--out`, the output is `dist/` under the config's
directory. Output must be absent. Wisp stages the whole build and package, then
moves the completed result into place. A compiler, composition, or packaging
failure leaves no completed output directory.

## Additional source languages

Use the packaged adapters directly through `build.compiler`, without first
creating a prebuilt Component project:

```toml
[package]
name = "demo:arithmetic"
version = "0.1.0"

[build]
compiler = "zig"
sources = ["arithmetic.zig"]
```

For example, `export fn add(a: i32, b: i32) i32 { return a + b; }` needs no
authored WIT. Wisp derives direct scalar exports from explicit Zig,
AssemblyScript, Nim, TypeScript, Bend 2, Swift, Java and Kotlin signatures. Swift uses
its Wasm export attributes; Java and Kotlin use TeaVM's `@Export` annotation.
Unsupported or ambiguous types require an explicit contract. Wisp does not
guess the types of unannotated JavaScript functions or Grain return values.

A `.wit` file beside an entrypoint, with the same stem, takes precedence over
signature inference. Wisp preserves its interface identities and imports and
creates a wrapper world with the release's identity. For another contract path,
set `build.wit` to a WIT file or package directory and select the world with `build.world`, or use named build
units with an explicit `world`. Authored WIT must use the release's identity
and version. The generated and selected contracts stay under `build/units/`.

| Compiler selection | Current Component API |
| --- | --- |
| `zig`, `assemblyscript`, `nim` | Explicit scalar ABI exports; Wisp wraps Core Wasm and initializes Nim's reactor |
| `javascript`, `typescript` | Authored WIT supports synchronous typed imports, strings, lists, records and results; TypeScript transpilation does not type-check |
| `goose` | Typed scalar `export fn` APIs; stack-free `scalar-v1` implementations with reactor initialization |
| `grain` | Raw scalar exports with an adjacent or authored WIT contract; heap values require ABI conversion |
| `bend` | Bend 2 direct `U32`/`F32` function exports through the JavaScript backend; sequential execution |
| `swift` | Scalar exports with matching Wasm export attributes; aggregates require ABI conversion |
| `java`, `kotlin` | TeaVM scalar exports; set `entry` to the main class, such as `Math` or `MathKt` |
| `python` | Default scalar `wisp:python/api@0.1.0` adapter, or multiple typed exports through an authored WIT contract and a separately provisioned runtime template |

Goose runs its frontend and C backend inside Wasm. Its initial `scalar-v1`
profile rejects implementations requiring dynamic data stacks, threads, queues
or native libraries. Public strings and compound types require future ABI
adapters. Use `WISP_GOOSE_TOOLCHAIN` or `build.tools.goose` to select its package.
The frontend requires a host with Wasm exception support. Set
`WISP_GOOSE_FRONTEND_HOST` or `build.tools.goose-frontend-host` when the shared
LLVM host lacks that feature. See the
[Goose example](../examples/build-goose/README.md).

For Python's fixed scalar profile, declare its WIT interface and use `entry` to
select the two-argument integer callable. For multiple exports, strings,
lists, records and typed results, provide `build.wit` and `build.world`, or an
adjacent WIT contract. Provision its matching runtime template in buildtools first
with `python3 python/bootstrap_contract.py --wit CONTRACT --world WORLD`.
The Python toolchain must include that template under `dist/contracts/`.
Normal application builds use Wasm-hosted compilation and do not run native
componentize-py. This contract path requires an import-free synchronous world;
resources and async APIs are unsupported. New templates bundle local Python modules. Set
`project` to the directory containing your top-level packages, and keep one
entrypoint in `sources`. Vendored pure Python dependencies can live under that
source root. Package initialization, relative imports and function-local imports
execute in guest CPython. Older templates need `bootstrap_contract.py --rebuild`
for module support. Native extensions and package data are not bundled.
Python functions use WIT names converted to snake_case. Records use the generated
`wit_world.exports` dataclasses, and typed errors use `componentize_py_types.Err`.
`entry` applies only to the default scalar adapter. JavaScript
has no Node builtins or browser DOM. The additional scalar adapters do not
automatically convert language-managed strings, collections or objects.

Set `project` to include local imports under a shared source root. Swift, Java
and Kotlin accept multiple source files. Source inputs, adjacent contracts,
adapter commands and output hashes are recorded in `build/build.json`.
Compilation and Component validation finish before the output is published.
Non-source assets such as Zig `@embedFile` inputs are staged and hashed too.
Source trees have a 32 MiB aggregate limit; symlinks and generated build/cache
directories are excluded or rejected.

Each language's tool path is its installed adapter directory or `compile.py`.
For example, `build.tools.zig` and `WISP_ZIG_TOOLCHAIN` select Zig. The other
keys use the compiler names above; JavaScript and TypeScript share one package.
`wisp toolchain env` exports the selected language and dependency paths.
Host Python is selected through `build.tools.python-host` or `WISP_PYTHON_HOST`.
`build.tools.python` selects the Python compiler package instead.

Run the direct source build checks after provisioning the compiler packages and
their shared dependencies:

```sh
python3 tests/source-build/languages.py --buildtools ../wisp-buildtools \
  --cli target/debug/wisp --out out/direct-languages --cache "$HOME/.cache"
```

The check blocks native compiler commands, invokes `wisp build` for every
additional language, calls `add(20,22)`, and verifies each build receipt's
compiler identity and Component hash. Use `--languages` to select a subset.
For Bend, first [bootstrap its compiler Component](../tools/bend/README.md#bootstrap).
The [Bend example](../examples/build-bend/README.md) builds without authored WIT.
Entrypoints containing proofs or nonnumeric helpers need an adjacent or authored
contract selecting the numeric exports. Local imports use `project`; hub imports,
foreign effects and heap-valued public types are unsupported. This backend
provides no worker pool or GPU execution.

## Infer an ordinary source API

Omit `build.wit` and `build.world` to infer the contract from public functions.
Wisp reads C++, Go and Rust signatures, generates WIT, then runs pinned
`wit-bindgen-cli 0.61.1`. Small source adapters convert native values to the
upstream generated types. The compiler still runs inside Wasm.

```toml
[package]
name = "demo:cpp-math"
version = "0.1.0"
[build]
compiler = "clang"
sources = ["math.cpp"]
```

```cpp
#include <cstdint>
uint32_t add(uint32_t a, uint32_t b) { return a + b; }
```

Rust uses ordinary `pub fn` functions with `Vec<T>`, `Option<T>`, fieldless enums,
records and `Result<T, E>`. Go uses exported functions, slices, optional pointers,
named integer enum constants, records and error results. C++ uses ordinary
functions with `std::vector`, `std::optional`, scoped enums, records and
`std::expected`. Nested values follow the same recursive conversion rules.
Unsupported or ambiguous public types fail during inference before compilation.

Public package-level Go functions, Rust `pub fn` functions, and non-static C++
free functions become exports. Private functions, anonymous C++ namespaces and
Go `main` remain internal. Native function bodies keep using their own language
libraries and installed dependencies.

Wisp discovers Components carried by installed Go and Rust packages and connects
their imports automatically. Go consumers pin modules in `go.mod` and stage them
with `GOOS=wasip1 GOARCH=wasm CGO_ENABLED=0 go mod vendor`. Rust consumers use the
provider's generated guest crate through ordinary Cargo path or vendor metadata.
Application builds remain offline.

Inferred records require named public fields. C++ inference reads declarations
in configured source files, so public record and enum definitions must be there.
Implementation code can use headers. Recursive types, borrowed Rust strings and
slices, public generics, conditional Rust exports, variadics and overloads remain
unsupported. Each source unit exposes 1 to 64 functions with up to 16 parameters.

## Supply an explicit WIT contract

Set `build.wit = "wit"` and `build.world = "library"` when the contract should be
authored directly. In that mode, implementations use the upstream generated API.
For example:

```wit
package demo:cpp-math@0.1.0;
world library { export add: func(a:u32,b:u32)->u32; }
```

```cpp
namespace exports::library {
uint32_t Add(uint32_t a, uint32_t b) { return a + b; }
}
```

Install the generator during tool setup, not application execution:

```sh
python3 tools/upstream-bindings/setup.py --out .tools/upstream-bindings
export WISP_WIT_BINDGEN="$PWD/.tools/upstream-bindings/wit-bindgen"
```

C and C++ use upstream `c` and `cpp` generators respectively. The compiler links
the generated metadata object and WASI SDK 34 target libraries inside Wasm.
C++23 types include `std::optional`, `std::expected`, generated records and enums,
and upstream `wit::string`/`wit::vector` ownership wrappers.

With authored WIT, Go implements functions in upstream `export_<namespace>_<package>_<interface>`
packages. Wisp generates interface packages and `wit_exports`, then imports the
export glue from a staged `main` package. `go.bytecodealliance.org/pkg v0.2.3`
supplies the exact runtime required by the pinned generator. Its authenticated
source ships with the Go toolchain and is staged offline. For an existing
workspace toolchain, set `WISP_GO_WIT_RUNTIME` to buildtools' `go/wit-runtime`.
Application compilation invokes only `compile.wasm` and `link.wasm`.

With authored WIT, Rust implementations include the staged `src/wisp_bindings.rs`, implement its
`Guest` trait, and invoke its `export!` macro. The root library compiles directly
to a cdylib. Inferred Rust APIs use a generated wrapper around the ordinary
root library.

General lists, options, enums and synchronous compound values follow upstream
bindings, including nested lists and optional enum values. Wisp's Rust runtime
projection is synchronous and excludes flags, resources and async handles.
Other source languages retain their documented adapter-specific ABI limits.

See [C](../examples/build-c), [C++](../examples/build-cpp),
[Go](../examples/build-go), [Rust](../examples/build-rust), and
[rich values](../examples/build-values) for ordinary source APIs; the C example uses authored WIT.

## Import a Go package from C++

C++ can consume an installed Component package through an ordinary header:

```cpp
#include <arithmetic.hpp>
unsigned doubled_sum(unsigned a, unsigned b) {
    return arithmetic::add(a, b) * 2;
}
```

Declare the package identity, exact version and installed release directory:

```toml
[dependencies.arithmetic]
package = "demo:go-arithmetic"
version = "0.1.0"
path = "deps/arithmetic"

[build]
compiler = "clang"
sources = ["cpp/main.cpp"]
```

Install the release with `wisp library fetch`. Wisp validates its Component and
language archives, checks the requested identity and version, generates
`arithmetic.hpp` from the exports, and composes the provider into the result.
The alias gives the header and C++ namespace their names. A single `api`
interface yields calls such as `arithmetic::add`; other interfaces add a nested
namespace. Functions use snake case, with a trailing underscore for C++ keywords.
No WIT or Wasm attributes appear in application code.

The provider can be built from Go or another supported source language. The C++
build uses the installed bytes and runs no Go compiler or adapter conversion.
A Go provider retains its WASI runtime requirements; the final Component exposes
those imports to its host. Wisp carries dependency notices into the output.
The generated headers support the synchronous value types above, including
records and typed results. Root package dependencies currently require inferred
C++ builds. Explicit contracts continue to use their existing unit graph.

[The C++ importing Go example](../examples/cpp-import-go/README.md) contains an
installed Go package, ordinary C++ code, generated inspection snapshots, and a
local publish/fetch demonstration that builds after all registries shut down.

## Local compiler workspace

Saved compiler bundles can be installed and selected without a source checkout
of the compilers. See [toolchain installation](toolchain-packages.md#install-and-select-compiler-versions).
`wisp build` reads the selected versions from `WISP_TOOLCHAIN_HOME` or Wisp's
default data directory. Explicit tool paths continue to take precedence.

The buildtools share one repository beside Wisp at `../wisp-buildtools/`.
Toolchain directories are `clang`, `go`, `rust`, `zig`, `assemblyscript`, `nim`,
`javascript`, `python` and `grain`. Shared LLVM packaging and Wasm utilities live in
`llvm` and `wasm-tools`. The source-built Rust compiler runs in Wasm and passes
unmodified Serde derives and basic build scripts.

From this Wisp checkout, select the relocated compiler artifacts with:

```sh
source ../wisp-buildtools/env.sh
```

That configures C/C++, Go, the Rust compiler and the native Go module installer.
Wisp provisions execution hosts separately from compiler packages. Explicit
host selections are preserved. The remaining composition and packaging tools
are configured below. This checkout's
local `.tools/build/env.sh` also selects them and works from any current
directory when sourced by its absolute path.

## Configure the compilers

Builds require these already provisioned tools. Paths can be in the config or in
environment variables. Environment variables take precedence.

```toml
[build.tools]
wasmtime = "/path/to/wisp/.tools/shared-hosts/wisp-wasmtime-50.0.0/wasmtime"
clang = "/path/to/wisp-buildtools/clang/.toolchains/build-p3/bin/clang-wasm.component"
llvm = "/path/to/wisp-buildtools/llvm/dist/shared"
go = "/path/to/wisp-buildtools/go/dist/go-toolchain-full-1.27.1"
rust = "/path/to/wisp-buildtools/rust/dist/compiler"
rust-wasmtime = "/path/to/wisp/.tools/shared-hosts/wisp-wasmtime-50.0.0/wasmtime"
preview1-adapter = "/path/to/wasi_snapshot_preview1.reactor.wasm"
```

| Config key | Environment variable | Used for |
| --- | --- | --- |
| `wasmtime` | `WISP_WASMTIME` | Running C/C++ and Go compilers; requires 50.0.0 |
| `clang` | `WISP_CLANG` | Driverless `clang-wasm.component` |
| `llvm` | `WISP_LLVM` | Shared LLVM 24 core modules, loaded into the Clang compiler Component |
| `go` | `WISP_GO_TOOLCHAIN` | Extracted full Go 1.27.1 toolchain package |
| `rust` | `WISP_RUST` | Rust compiler bundle with verified compiler, runner and sysroot |
| `rust-wasmtime` | `WISP_RUST_WASMTIME` | Wasmtime 50.0.0 for WASIp3 Rust |
| `preview1-adapter` | `WISP_PREVIEW1_ADAPTER` | Wasmtime 50.0.0 reactor adapter for Go and Rust output |

The execution hosts are Wisp dependencies shared by compiler packages.
For the 50.0.0 compiler path, Wisp prefers its matching `wisp-worker`, found beside
`wisp`, on `PATH`, or through `WISP_WORKER`. The worker embeds upstream Wasmtime
50.0.0 and handles shared core-module imports through its library API. Keep both
executables together when installing or snapshotting the CLI. The configured
CLI host remains a fallback when no worker is available. There is
no lookup inside the Rust compiler bundle for a bundled Wasmtime executable.
Rust 1.99.1 is a WASIp3 compiler Component with LLVM 24 linked into it. It uses
Wasmtime 50.0.0. Wisp checks the compiler manifest and execution host before
compilation. `WISP_RUST_WASMTIME` overrides the host; the compiler can also use
`WISP_WASMTIME`.

Clang's Component imports LLVM and its shared C/C++ runtime modules from the
separate LLVM package. Wisp verifies the shared package manifest, hashes, module
paths and sizes, then supplies the core modules at instantiation with
the worker's module-import request. The fallback CLI uses `--preload`, which
requires Wisp's patched Wasmtime 50.0.0 host. Build that fallback with:

```sh
python3 tools/shared-hosts/build.py --jobs 2
export WISP_WASMTIME="$PWD/.tools/shared-hosts/wisp-wasmtime-50.0.0/wasmtime"
```

This host is installed once for Wisp. It is excluded from compiler packages.
The build receipt records the shared LLVM manifest and module hashes.

AssemblyScript's compiler adapter also needs the Wasmtime Python host API,
version 50.0.1, which Wisp checks before use. Install that binding in the host Python environment or set
`WISP_WASMTIME_PYTHON` to the directory containing its `wasmtime` module. It is
shared host infrastructure and is excluded from the AssemblyScript package.
`wisp toolchain hosts` reports the selected runtimes and their hashes.

`WISP_GO` selects the native Go module installer. The Wasm-hosted compiler uses
`WISP_GO_TOOLCHAIN`.

Tool paths in TOML are relative to the config's directory, or absolute. Only the
tools needed by the selected units are required. Wisp checks the Go toolchain's
complete `SHA256SUMS` inventory and the reactor adapter's pinned SHA-256. The
build receipt records the Wasmtime and C++ compiler executable hashes. There is
no automatic compiler installation or rebuilding of LLVM during a project build.

For a graph with more than one unit, provision the existing pinned WAC 0.10.1
composer and set `WISP_WAC`. Its checksum is checked by Wisp's dependency resolver.
Packaging also needs the existing pinned Node 24.20.0, JCO 1.33.0, authenticated
npm cache, and populated Cargo dependency cache. Set `WISP_NODE`, `WISP_JCO`, and,
when needed, `WISP_NPM_CACHE` as described in
[component-library-packages.md](component-library-packages.md). The Node directory
must be on `PATH` for `npm pack`. Packing remains offline; seed the dependency
caches before building.

Native Go, Clang, or LLVM executables are not used to compile project code. The
Go adapter is part of the Rust CLI and invokes the Go toolchain's `compile.wasm`
and `link.wasm` directly. Python is used by the acceptance runner, not by `wisp build`.

## C++ editor support

Point the editor at WASI SDK's target include directories and the staged
`build/units/<unit>/bindings` directory. Generated declarations are ordinary
C/C++ headers. C++ builds use `-std=c++23`, disabled exceptions and RTTI.

## Compose build units

Declare a source-unit graph when an implementation imports another Component.
Without `build.wit`, Wisp infers each unit’s exports and discovers installed
Component dependencies. With `build.wit`, each unit selects an explicit world
from the shared package.

This is the configuration in [the explicit mixed example](../examples/build-mixed):

```toml
[package]
name = "demo:mixed-math"
version = "0.1.0"

[metadata]
description = "A Go library calling a C++ Component dependency"
license = "MIT"

[build]
root = "application"
wit = "wit"

[build.units.arithmetic]
compiler = "clang"
world = "math"
sources = ["math.cpp"]
optimization = "O1"

[build.units.application]
compiler = "go"
world = "application"
project = "go"

[build.units.application.dependencies]
"demo:mixed-math/arithmetic@0.1.0" = "arithmetic"
```

The shared `wit/world.wit` defines each unit's imports and exports:

```wit
package demo:mixed-math@0.1.0;
interface arithmetic { add: func(a: u32, b: u32) -> u32; }
interface calculator { doubled-sum: func(a: u32, b: u32) -> u32; }
world math { export arithmetic; }
world application { import arithmetic; export calculator; }
```

Dependency keys are exact WIT import names. Their values name other build units.
Wisp rejects unknown units, cycles, missing declarations, and unused units.
Every unit must be reachable from `build.root`. The resolver and WAC check the
compiled interface signatures before packaging the root. The result embeds its
Component dependencies; execution does not fetch them.

Package identity and version must agree with the WIT package. Source and WIT
paths must stay inside the config's directory. Units use different worlds in
that WIT package. All units contribute to one root package.

## Generated imports and exports

All C/C++/Go/Rust build units use upstream bindings. Imported WIT interfaces
become the upstream generator's typed namespaces or packages. Export
implementations must match the generated signatures. Generated files live in
`build/units/<unit>/bindings` or in the staged Go/Rust source tree.

Cross-language calls use the same WIT types and canonical ownership rules.
Dependency keys name exact WIT interface imports; WAC connects them to the
selected provider. Installed Go guest modules also carry their provider
Component and upstream-generated rich imports. Native Wago modules retain a
separate scalar capability check.

## Import a prebuilt Component

A build unit can consume an existing `.wasm` file instead of invoking a
compiler. Set `component` with its path and SHA-256, and select the WIT world
that describes its imports and exports. Omit `compiler`, `sources`, `includes`,
`project`, and `optimization` on that unit.

```toml
[build.units.arithmetic]
world = "prebuilt-arithmetic"
component = { path = "deps/arithmetic.wasm", sha256 = "<64 lowercase hex characters>" }
```

A normally installed Go module can provide the Component too. Declare the
provider world and exact import mapping in the unit graph:

```toml
[build.units.arithmetic]
world = "prebuilt-arithmetic"
component = { module = "example.org/cpp-arithmetic" }
```

Publish its producer with `--go-module example.org/cpp-arithmetic`, add a
versioned `require` and normal import to the consumer's Go project, then install
with `go mod download` and `GOOS=wasip1 GOARCH=wasm CGO_ENABLED=0 go mod vendor`.
Wisp resolves the Component from that module's vendor directory, verifies its
version and regenerated guest bindings, and compiles the imported Go package
with the Go compiler. Registry access and native Go are needed during module
installation;
`wisp build` uses the vendored sources offline. No `replace` directive is needed.

Other units reference it through their `dependencies` tables in the same way as
a source-built unit. The file must be inside the project directory and already
be an executable WebAssembly Component. A raw Core Wasm module needs WIT and
Component assembly first, for example through a separate
`wisp build --component-only` of its producer project.

Wisp checks the file hash, validates the binary, and compares its imports and
exports with the selected WIT world before running any compiler. This contract
comparison is exact. Include imported interface definitions under `wit/deps`
when the dependency has a different WIT package identity. Prebuilt units retain
the original bytes and hash in the build receipt and run no compiler commands.
Only source units require compiler tools; consuming a C++ Component
does not require the C++ compiler.

[The Go import example](../examples/go-import-wasm/README.md) imports a published
C++ package as an ordinary Go module. Its included vendor directory contains
real installed bindings and the C++ Component. `registry-demo.py` exercises
publication, installation into empty caches, native Go execution, and an offline
Go-to-Wasm build using the same application code.

### Build a Grain Component

The [Grain example](../examples/build-grain/README.md) compiles Grain with the
Wasm-hosted compiler in `../wisp-buildtools/grain`, assembles its explicit WIT
contract with the shared Wasm tools package, and admits the resulting Component
through a prebuilt unit:

```sh
source .tools/build/env.sh
python3 examples/build-grain/build.py --out out/grain-math
"$WISP_WASMTIME" run --invoke 'add(20,22)' out/grain-math/component.wasm
```

Set `WISP_GRAIN_TOOLCHAIN` to use another installed Grain package directory.
`WISP_WASMTIME` provides its shared execution host. The adapter also accepts
`WISP_PREVIEW1_ADAPTER` for retained WASI Preview 1 imports. The example retains
its Grain source and compiler assembly receipt under `build/grain` in the output.
Add `--package` to generate the usual language packages after Component admission.

This path uses authored WIT and raw scalar Grain types such as `WasmI32` and
`WasmI64`. Grain initializes its globals through the reactor entry point after
WASI adapter setup and before Component exports run. Boxed Grain `Number`, strings
and collections need an explicit representation conversion at the public boundary.
C++/Go/Rust APIs can infer WIT from ordinary functions or use an authored
contract. Both paths use upstream-generated bindings.

## Output and current limits

Default output contains `library-release.json`, `component.wasm`, `rust.crate`,
`node.tgz`, and their inspection directories. `--go-module example.org/library`
also emits a Go module. Scalar APIs select native Wago or `wasip1/wasm`
imports automatically with the same public functions. The portable module
includes the Component and both implementations, and normal Go tooling installs
the runtime dependency. `--go-guest-module` emits rich synchronous guest bindings when the portable
scalar profile does not apply. `--go-module` retains native Go clients for
richer contracts. Go implementation builds preserve the packaged Go runtime license in
the release notices. The consumer package option is independent of which
compiler builds the library implementation.

`build/build.json` records the config, contract mode, source and tool hashes,
compiler commands, per-unit Component hashes, and final Component hash. Each
unit retains its generated or selected WIT contract. The
`build/units/` directories retain staged sources and generated bindings; a mixed
build also retains its dependency lock and composition report. This is local
build evidence. Publication transports the existing admitted Component and
language archives, rather than the source snapshots.

`--component-only` stops before language packaging and produces a Component
directory suitable for a later `wisp library pack`. This is useful for testing
compiler integration without the packaging tools.

C/C++ builds require a WASI SDK target sysroot selected by
`build.tools.clang-wasi-sysroot`, `WISP_CLANG_WASI_SYSROOT`, or the installed
Clang package. Wisp mounts its SDK root at `/wasi-sdk` and passes the fixed
`wasm32-wasip1` target to the Wasm-hosted driver. The SDK supplies resource
headers, wasi-libc, no-exceptions libc++, libc++abi and compiler-rt. No native
compiler is used for application code. Startup uses `crt1-reactor.o` and
`_initialize`, including C++ global constructors. A unit's sources are combined
into one translation unit; generated metadata objects link in-process.

Go supports local multi-file and multi-package projects, WASIp1 build selection,
and installed vendor dependencies. It does not fetch modules or invoke native
Go during application compilation. The upstream runtime owns canonical
allocation and pinning. Wasmtime's pinned Preview 1 adapter supplies the clock
pause hook needed during canonical allocation. The bespoke 4 MiB arena and its
post-return rewriter have been removed.

## Verification

Run the CLI regression tests and upstream guest package checks:

```sh
cargo test --locked -p wisp --test build --test library_go_upstream
```

The source runners build component-only C/C++/Go/Rust implementations and a
mixed Go-to-C++ graph, then execute the Components through JCO. They do not
install npm packages or rebuild native library hosts.

```sh
python3 tests/source-build/run.py --cli target/debug/wisp \
  --out out/source-apis --runtime-cache .wisp-cache
python3 tests/source-build/values.py --cli target/debug/wisp --out out/source-values
```

The shared rich verifier checks general string lists, enum options, nested lists
of options, records, typed results and repeated ownership transfers. The Serde
runner separately checks procedural macros inside the Wasm-hosted Rust compiler.
[Source compiler CI](source-build-ci.md) describes provisioning and receipts.
Existing library-chain suites cover registry publication and native consumers.

## Rust compiler

Rust uses the verified Wasm-hosted compiler bundle and its locked offline Cargo
metadata. Wisp stages upstream synchronous bindings. Inferred APIs use a generated
wrapper; authored APIs compile the root library directly as a cdylib. Compiler, runner, sysroot and execution hashes
remain in the build receipt. Native Cargo resolves metadata; it does not compile
the application.

`wisp library rust-guest ADMITTED_RELEASE --out SDK_DIR` creates an import-only
crate from the provider's WIT using the same pinned upstream Rust generator.
Its APIs follow upstream package/module names and ownership rules. It contains
no embedded host runtime. Resources, async handles and flags remain excluded
from this synchronous source projection.
