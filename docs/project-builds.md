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
| `python` | The packaged `wisp:python/api@0.1.0` interface, including `add(s32, s32) -> s32`; `entry` chooses the annotated Python callable |

Goose runs its frontend and C backend inside Wasm. Its initial `scalar-v1`
profile rejects implementations requiring dynamic data stacks, threads, queues
or native libraries. Public strings and compound types require future ABI
adapters. Use `WISP_GOOSE_TOOLCHAIN` or `build.tools.goose` to select its package.
The frontend requires a host with Wasm exception support. Set
`WISP_GOOSE_FRONTEND_HOST` or `build.tools.goose-frontend-host` when the shared
LLVM host lacks that feature. See the
[Goose example](../examples/build-goose/README.md).

Python's installed runtime currently has a fixed public interface. Selecting a
different callable does not change that interface. Multiple arbitrary Python
exports and general WIT bindings require a newer runtime adapter. JavaScript
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

## Write ordinary application code

A Go project needs only its source and a compiler selection:

```toml
[package]
name = "demo:application"
version = "0.1.0"

[build]
compiler = "go"
project = "go"
```

Public package-level Go functions define its exports. Implement ordinary names
such as `DoubledSum`; Wisp parses their signatures, derives the contract, and
generates the export glue. Private functions and `main` stay internal. Function
bodies can use local packages, installed Go dependencies and supported standard
library features. For C++:

```toml
[package]
name = "demo:arithmetic"
version = "0.1.0"

[build]
compiler = "clang"
sources = ["arithmetic.cpp"]
```

Write ordinary functions such as
`unsigned add(unsigned a, unsigned b) { return a + b; }`. Wisp derives the API
from non-static free-function definitions, including namespace-qualified
functions. `static` functions and anonymous namespaces stay internal.
Wisp generates the WIT and ABI bindings. Application functions need no special
prefix or compiler directives. Rich C++ signatures use the generated
`wisp_types.hpp` public value types described below.

When Go imports a Wisp-generated package, Wisp discovers the Component through
the installed module's metadata and wires it automatically. Publish scalar Go
modules with `--go-module MODULE`; each module works in native Go and Wasm
builds with the same imports and function calls. Native builds link the
Wago runtime automatically. Pin the version in
`go.mod`, install with `go mod download`, and run
`GOOS=wasip1 GOARCH=wasm CGO_ENABLED=0 go mod vendor`. The compiler builds from
that vendor directory offline. The [registry example](../examples/go-import-wasm/README.md)
shows ordinary C++ packaged and imported by ordinary Go, with both contracts
and their composition generated by Wisp.

Inference supports scalars, strings, byte buffers, plain records and
synchronous results. Rust uses `String`, `Vec<u8>`, public structs and
`Result<T, E>`. Go uses `string`, `[]byte`, exported structs and ordinary
`(T, error)` returns, which become `result<T, string>`. A sole Go `error` becomes
`result<_, string>`. Go also supports local types and aliases, including its
64-bit `int` and `uint` on the pinned Wasm target.

Inferred records must have named public fields. Rust and Go records must expose
every field, and Go embedded fields are unsupported. C++ API structs must be
defined in the configured source files; inference does not read their declarations
through `#include`. Headers remain available to the compiler for implementation
code. C++ inference recognizes `using` aliases, not arbitrary C-style `typedef`
declarations. Rust public re-exports and conditional public declarations are
unsupported.

Freestanding C++ uses plain structs and the generated `wisp_types.hpp` header.
`wisp::string` and `wisp::bytes` accept a pointer and length; string literals
also work. `wisp::result<T, E>::success(value)` and `failure(error)` construct
results. These types require no C++ standard library. Imported strings and byte
buffers own their returned data and release it when the last copy is destroyed.
Exported C++ arguments are borrowed for the duration of that call. To retain
an argument across calls, use `wisp::string::copy(value.data, value.size)` or
`wisp::bytes::copy(...)`. Returning an argument directly is safe because Wisp
copies the result before releasing the incoming buffers.

Rust `()` and C++ `wisp::unit` or `void` result payloads map to WIT unit values.
Results can be nested. Go failures use the error's string message rather than a
separate typed error payload.

The [rich value examples](../examples/build-values/) exercise all three
languages. Wisp copies values across Component boundaries, releases incoming
allocations after conversion, and emits post-return cleanup for owned results.
Records can contain supported values, including nested records and results.
Recursive records, pointers, borrowed Rust strings/slices, variadics, overloads,
public generics, resources and async signatures receive source-level errors.
An inferred source unit exports 1 to 64 functions with at most 16 source
parameters each. Inferred records contain 1 to 64 fields; a record parameter can
still flatten to more than 16 canonical ABI values.
Generated contracts and bindings remain under `build/units/` for inspection.

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

## C++ editor support in this repository

The C++ examples include portable `compile_flags.txt` files and the generated
headers needed by their sources. Clangd discovers the project flags, selects
`wasm32-unknown-unknown` and resolves the same public API used by Wisp builds.
Opening the checkout requires no initial compilation or machine-specific include
paths. This uses clangd's standard
[project compilation flags](https://clangd.llvm.org/installation.html#project-setup).

Inspection wrapper snapshots use checkout-relative includes so they also parse
in editors. Actual build outputs retain the compiler sandbox paths. Generated
header snapshots should be refreshed from a successful build when dependency
versions or contracts change. They supply editor types; Wisp derives and checks
the actual compilation bindings from the installed Component or contract.

Verify diagnostics with the editor's clangd executable:

```sh
python3 tests/source-build/editor.py --clangd /path/to/clangd \
  --out out/cpp-editor-check
```

The check covers all C++ example sources and headers in the checkout and a
relocated copy containing no outputs or toolchains. A deliberately invalid
numeric API call must produce a type error.

## Explicit contracts for advanced builds

Existing Component integration workflows can select an explicit WIT world and
declare a source-unit graph. This is optional. The ordinary Go and C++ workflow
above derives these details automatically. Use `build.wit` for an authored
contract, or the existing `world`-based units with a `wit/` directory.

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

## Bindings in explicit-contract mode

In explicit-contract mode, Wisp generates `wisp.h` for C/C++. Implement `exports_<interface>_<function>`;
call `imports_<interface>_<function>` for imported functions. Names use snake
case. A top-level world function has no interface prefix.

```cpp
#include "wisp.h"
unsigned exports_arithmetic_add(unsigned a, unsigned b) { return a + b; }
```

For Go, Wisp generates `wisp_bindings.go` in a staged copy of the main package.
A project can check in that exact generated file to expose the bindings beside
its implementation. Wisp checks it against the generated bytes before compiling
and rejects stale or edited copies. The [prebuilt import example](../examples/go-import-wasm/README.md#inspect-what-wisp-generated)
includes generated Go import/export declarations, C++ export glue, composition
source, and the final Component's complete WIT import inventory. Its ordinary
application code uses inferred contracts rather than checked-in binding inputs.
To refresh a checked-in file after changing WIT or upgrading the generator, move
it aside, build, then
copy `build/units/<unit>/source/wisp_bindings.go` from the output back into the
Go project.

Implement `Export<Interface><Function>` and call `Import<Interface><Function>`.
Names use Pascal case; WIT scalar arguments become their corresponding Go types.
The example has a local `internal/scale` package too.

```go
package main
import "example.org/mixed-math/internal/scale"
func ExportCalculatorDoubledSum(a, b uint32) uint32 {
    return scale.Double(ImportArithmeticAdd(a, b))
}
func main() {}
```

Wisp leaves the project's source files untouched. The generated Go wrappers use
`//go:wasmexport` and `//go:wasmimport`. The linker produces a WASIp1 reactor;
Wisp reserves adapter storage inside Go static data, installs a small allocator
that works before the Go runtime initializes, embeds the selected WIT, and
adapts the reactor into a Component. Reserving that storage prevents Go heap
growth from overwriting the adapter state. The C++ compiler emits
freestanding Core Wasm, which Wisp also wraps using the selected WIT.

## Export Go that calls C++ Wasm

The mixed example builds the C++ arithmetic unit, imports its WIT interface into
Go, and compiles the Go unit. Wisp composes the two Components
into one `component.wasm` exporting the Go `calculator` interface. The arithmetic
dependency is embedded in that file.

With the tools configured, build and call the Go export directly:

```sh
wisp build examples/build-mixed --component-only --out out/go-imports-cpp
"$WISP_WASMTIME" run --invoke 'doubled-sum(20, 1)' out/go-imports-cpp/component.wasm
# 42
```

Go calls `ImportArithmeticAdd(20, 1)`, implemented by C++, then doubles the
result. The output is a WebAssembly Component and needs a Component-capable
runtime with WASI support. Omit `--component-only` to also generate the Cargo
and npm packages for publication.

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

In explicit-contract mode, a normally installed Go module can provide the
Component too. In inferred builds this selector and its import mapping are
derived automatically from the Go import graph:

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
Ordinary source API inference remains available for Go, C/C++ and Rust.

## Output and current limits

Default output contains `library-release.json`, `component.wasm`, `rust.crate`,
`node.tgz`, and their inspection directories. `--go-module example.org/library`
also emits a Go module. Scalar APIs select native Wago or `wasip1/wasm`
imports automatically with the same public functions. The portable module
includes the Component and both implementations, and normal Go tooling installs
the runtime dependency. `--go-guest-module` remains a compatibility spelling
for this scalar profile. Richer contracts use the existing native Go client
package. Go implementation builds preserve the packaged Go runtime license in
the release notices. The consumer package option is independent of which
compiler builds the library implementation.

`build/build.json` records the config, contract mode, source and tool hashes,
compiler commands, per-unit Component hashes, and final Component hash. Each
unit retains its generated or selected WIT contract. The
`build/units/` directories retain staged sources and generated bindings; a mixed
build also retains its dependency lock and composition report. This is local
build evidence. Publication transports the existing admitted Component and
language archives, rather than the source snapshots.

`--component-only` stops before language packaging and produces an authored
directory suitable for a later `wisp library pack`. This is useful for testing
compiler integration without the packaging tools.

The source binding generator supports synchronous freestanding functions with
scalars, strings, `list<u8>`, records and results. It implements canonical
memory layouts, variant payload joins, indirect parameters beyond 16 flattened
values, and post-return cleanup. Other list elements, resources, async functions
and unsupported public types fail before compilation. Freestanding C source
retains its scalar-only bindings; the richer C-family bindings require C++.
Generated type names that would collide across different interface records fail
explicitly rather than selecting one record definition.

Installed Rust guest crates and generated C++ dependency headers can import
these rich APIs. The portable native/Wasm Go dependency projection remains
scalar-only. This limit does not restrict the rich APIs a Go source project can
export or their generated Rust and Node consumer packages.

The C++ compiler supports freestanding C and C++. A unit's `sources` are included
into one translation unit; they must all use the same language. `includes` adds
project header directories. The compiler does not provide libc++, exceptions,
RTTI, or threads. `optimization` applies only to C++, with `O0` as the default
and `O1` as the other supported value.

The Go compiler supports local multi-file, multi-package Go projects with the
packaged standard library. It selects WASIp1/wasm files and Go build constraints. It does
not fetch Go modules during compilation, use cgo, run generators, or support
local assembly and `go:embed` inputs. Install and vendor external modules before
building. The root must contain `package main`.

Go reactors use the pinned standard WASI 0.2.12 host profile. Rust and Node hosts
provide real clocks and secure randomness, with empty arguments and environment,
closed input, discarded output, and no filesystem preopens or inherited network.
Browser execution of that profile is explicitly excluded. Freestanding C/C++
packages retain the existing ordinary library targets.

Rich Go bindings reserve a separate 4 MiB arena for transient canonical values.
The host-facing allocator and post-return cleanup run as pure core Wasm, so Go's
allocator and garbage collector cannot invoke WASI while host calls are forbidden.
The arena is shared by the live canonical argument and result buffers of one
instance. It does not overlap Go's heap or the adapter's startup storage.
Exhaustion traps; 4 MiB is not a per-string allowance. Ordinary Go application
allocations continue to use its garbage-collected heap.

`--cache DIRECTORY` selects the Wasmtime machine-code cache; it defaults to
`.wisp-cache/` in the project. This cache speeds compiler startup. It does not
cache project outputs. Cache and output directories must be separate.

## Verification

Run the CLI regression tests with `cargo test --locked -p wisp --test build`.
With the tool environment above, the executable acceptance runner builds the
C++, Go, mixed, Go-importing-C++, and C++-importing-Go examples, publishes to disposable
Cargo/npm/OCI registries, fetches and compares the OCI release, then executes
installed npm consumers after
the registry shuts down:

```sh
python3 tests/source-build/run.py --cli target/debug/wisp \
  --out out/source-build-acceptance --runtime-cache .wisp-cache
```

Run rich value and installed dependency checks with:

```sh
python3 tests/source-build/values.py --cli target/debug/wisp --out out/source-values
python3 tests/source-build/values-interop.py --cli target/debug/wisp \
  --release out/source-values/go-release --out out/source-values-imports
python3 tests/source-build/values-edge.py --cli target/debug/wisp \
  --out out/source-values-edge
```

The edge suite checks nested results, unit payloads and records containing
results through actual C++ and Rust source builds. With the optional Wasmtime
Python API configured, `tests/source-build/cpp-memory.py` checks allocator reuse
against `out/source-values/cpp-release/build/units/application/work/core.wasm`.

[Source compiler CI](source-build-ci.md) documents clean-prefix acquisition,
Serde macros, cross-language composition, installed native Rust execution and
the required compiler artifact manifest.

The runner freezes source copies and writes `result.json` only after all
selected cases pass. Repeat `--case` to select individual examples. It also
verifies that a real compiler failure does not leave a published output directory.


## Rust compiler

Select an ordinary Cargo library project:

```toml
[package]
name = "demo:rust-math"
version = "0.1.0"

[build]
compiler = "rust"
project = "rust"
```

Public Rust functions define the exported API. Public modules, aliases, owned
strings and bytes, public structs and typed results are supported. Function bodies
may use `std`, allocation and generic private helpers. Public generic, async,
unsafe or conditional functions need an ordinary supported wrapper.

The source-built Rust compiler supports Serde derives. Build
with the normal Wisp command:

```sh
source .tools/build/env.sh
target/debug/wisp build examples/build-rust --out out/my-rust-release
```

For only the composed Component:

```sh
target/debug/wisp build examples/build-rust --component-only --out out/my-rust-component
"$WISP_WASMTIME" run --invoke 'add(20, 22)' out/my-rust-component/component.wasm
```

The compiler itself uses Wasmtime 38 because its legacy WASIp1 thread imports
were removed from Wasmtime 49's CLI. Application Components continue to use the
normal Wisp runtime. The build hashes the compiler, runtime, sysroot inventory
and runner and records the actual Wasmtime compiler invocations. Native Cargo
resolves locked offline metadata; it does not compile the application.

The source-built Rust 1.83.0-dev compiler supports `std`, base64 0.22.1,
basic `build.rs` scripts, procedural macros and Serde derives. See the
[compiler project](https://github.com/wisp-sh/wisp-buildtools/tree/main/rust),
[macro implementation notes](https://github.com/wisp-sh/wisp-buildtools/blob/main/rust/docs/proc-macros.md)
and [Wisp Serde verifier](../tests/source-build/rust-serde.md).

For a fresh compiler checkout, follow the
[source build instructions](https://github.com/wisp-sh/wisp-buildtools/blob/main/rust/docs/source-build.md)
to create the macro-capable bundle. `scripts/bootstrap.py` installs a separate
legacy compiler that supports the simpler cases but rejects procedural macros.
It does not install this source-built bundle.

The compiler host requires Linux x86_64. Basic build scripts can read sources,
generate files and emit supported Cargo settings; spawning native tools is
unsupported. Macro guests have no filesystem or process API. Compiler errors
currently print the expected diagnostic followed by a Wasm abort backtrace and
exit 134. The runner supports a Cargo subset, described in the compiler project.

`wisp library rust-guest ADMITTED_RELEASE --out SDK_DIR` derives a dependency-free
Cargo crate from an already admitted library release. Its ordinary Rust
functions call generated Wasm imports. It includes the Component, generated
bindings and notices, and Wisp authenticates those files again when building a
consumer. The checked-in generated signatures also typecheck in a host editor;
executing these guest bindings requires `wasm32-wasip1`. They support synchronous
scalars, strings, byte buffers, records and results. Resources, async values and
other list elements remain unsupported in this projection. It is separate from
the `wasm32-unknown-unknown` guest implementation inside the ordinary
`rust.crate`, and does not change that archive's native Rust API.


The Rust examples are [ordinary Rust](../examples/build-rust/README.md),
[Go importing Rust](../examples/go-import-rust/README.md),
[C++ importing Rust](../examples/cpp-import-rust/README.md), and
[Rust importing Go and C++](../examples/rust-import-wasm/README.md).
The last example includes a disposable-registry verifier for all four import
directions. Ordinary Cargo dependencies may use target-specific tables, including
the `cfg(any())` entries published by Serde. Wisp checks every such table for
Component imports, including imports reached through local or vendored wrapper
crates. Target-specific, optional and build-script dependencies that reach a
Component receive explicit errors. Package APIs use the synchronous types above.
